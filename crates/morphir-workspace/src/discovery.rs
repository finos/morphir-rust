//! Canonical, filesystem-free workspace discovery.

use std::collections::{BTreeMap, BTreeSet};

use globset::{GlobBuilder, GlobMatcher};
use morphir_config::{builtin_defaults, env_config_value, merge_all, parse_config};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    DiagnosticSeverity, DiscoveryFailure, DiscoveryRequest, DiscoveryResponse, FileTree,
    ProjectSnapshot, ProjectState, RelativePath, WORKSPACE_CONFIG_AMBIGUOUS,
    WORKSPACE_CONFIG_INVALID, WORKSPACE_CONFIG_MISSING, WORKSPACE_DISCOVERY_PROTOCOL,
    WORKSPACE_MEMBER_DUPLICATE_NAME, WORKSPACE_MEMBER_INVALID, WORKSPACE_PATH_NOT_CONFINED,
    WORKSPACE_PROTOCOL_UNSUPPORTED, WorkspaceDiagnostic, WorkspaceDiscoveryDetails,
    WorkspaceSnapshot, WorkspaceState,
    config::{found_adjacent_user_candidates, found_primary_candidates},
};

#[derive(Debug, Default, Deserialize)]
struct WorkspaceView {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    members: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    default_member: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProjectView {
    name: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default = "default_source_directory")]
    source_directory: String,
}

struct Layer {
    path: RelativePath,
    value: Value,
}

struct MemberProblem {
    message: String,
    path: Option<RelativePath>,
}

struct MemberConfigLayers<'a> {
    system: &'a Value,
    global: &'a Value,
    shared_workspace: &'a Value,
    member_primary: &'a Value,
    shared_workspace_user: &'a Value,
    member_user: Option<&'a Value>,
    environment: &'a Value,
    cli_overlay: &'a Value,
}

enum ProjectDecodeError {
    Invalid(String),
    NotConfined(String),
}

/// Discovers a Morphir workspace from portable, root-confined inputs.
#[must_use]
pub fn discover(request: DiscoveryRequest) -> DiscoveryResponse {
    match discover_internal(request, None) {
        Ok(snapshot) => DiscoveryResponse::Success { snapshot },
        Err(error) => DiscoveryResponse::Failure { error },
    }
}

/// Discovers a workspace and returns the exact merged configuration values
/// produced by the same pass as [`discover`].
pub fn discover_with_details(
    request: DiscoveryRequest,
) -> Result<WorkspaceDiscoveryDetails, DiscoveryFailure> {
    let mut collector = DetailsCollector::default();
    let snapshot = discover_internal(request, Some(&mut collector))?;
    Ok(collector.finish(snapshot))
}

trait EffectiveConfigCollector {
    fn root(&mut self, effective: &Value);
    fn project(&mut self, path: &RelativePath, effective: &Value);
}

#[derive(Default)]
struct DetailsCollector {
    root_effective: Option<Value>,
    root_is_project: bool,
    project_effective: BTreeMap<RelativePath, Value>,
}

impl EffectiveConfigCollector for DetailsCollector {
    fn root(&mut self, effective: &Value) {
        self.root_effective = Some(effective.clone());
    }

    fn project(&mut self, path: &RelativePath, effective: &Value) {
        if path == &RelativePath::root() {
            self.root_is_project = true;
        } else {
            self.project_effective
                .insert(path.clone(), effective.clone());
        }
    }
}

impl DetailsCollector {
    fn finish(mut self, snapshot: WorkspaceSnapshot) -> WorkspaceDiscoveryDetails {
        let root_effective = self
            .root_effective
            .expect("successful detailed discovery collects the root config");
        if self.root_is_project {
            self.project_effective
                .insert(RelativePath::root(), root_effective.clone());
        }
        WorkspaceDiscoveryDetails {
            snapshot,
            root_effective,
            project_effective: self.project_effective,
        }
    }
}

fn discover_internal(
    request: DiscoveryRequest,
    mut collector: Option<&mut dyn EffectiveConfigCollector>,
) -> Result<WorkspaceSnapshot, DiscoveryFailure> {
    if request.protocol_version != WORKSPACE_DISCOVERY_PROTOCOL {
        return Err(failure(
            WORKSPACE_PROTOCOL_UNSUPPORTED,
            format!(
                "unsupported workspace discovery protocol {}; supported version is {}",
                request.protocol_version, WORKSPACE_DISCOVERY_PROTOCOL
            ),
            None,
        ));
    }

    let root = RelativePath::root();
    let workspace_primary = required_layer(&request.development_root, &root, "workspace root")?;
    let workspace_user = optional_user_layer(&request.development_root, &workspace_primary.path)?;
    let system = optional_mount_layer(request.system_config.as_ref(), "system configuration")?;
    let global = optional_mount_layer(request.morphir_home.as_ref(), "Morphir Home")?;
    let empty = Value::Object(Map::new());
    let system_value = system
        .as_ref()
        .map(|layer| without_project_or_workspace(&layer.value));
    let global_value = global
        .as_ref()
        .map(|layer| without_project_or_workspace(&layer.value));
    let shared_workspace_user = workspace_user
        .as_ref()
        .map(|layer| without_project_or_workspace(&layer.value));
    let environment = env_config_value(
        "MORPHIR",
        request
            .environment
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    let workspace_effective = merge_all([
        &builtin_defaults(),
        system_value.as_ref().unwrap_or(&empty),
        global_value.as_ref().unwrap_or(&empty),
        &workspace_primary.value,
        workspace_user
            .as_ref()
            .map(|layer| &layer.value)
            .unwrap_or(&empty),
        &environment,
        &request.cli_overlay,
    ]);
    let workspace = decode_workspace(&workspace_effective, &workspace_primary.path)?;
    let root_has_project = workspace_effective.get("project").is_some();

    let member_directories = member_directories(
        &request.development_root,
        &workspace_primary.path,
        &workspace.members,
        &workspace.exclude,
    )?;
    let mut projects = Vec::new();
    if let Some(collector) = collector.as_deref_mut() {
        collector.root(&workspace_effective);
    }
    if root_has_project {
        projects.push(decode_root_project(
            &workspace_effective,
            &workspace_primary.path,
        )?);
        if let Some(collector) = collector.as_deref_mut() {
            collector.project(&root, &workspace_effective);
        }
    }

    let shared_workspace = without_project_or_workspace(&workspace_primary.value);
    for directory in member_directories {
        if directory == root {
            continue;
        }
        if let Some(project) = discover_member(
            &request.development_root,
            &directory,
            system_value.as_ref().unwrap_or(&empty),
            global_value.as_ref().unwrap_or(&empty),
            &shared_workspace,
            shared_workspace_user.as_ref().unwrap_or(&empty),
            &environment,
            &request.cli_overlay,
            &mut collector,
        ) {
            projects.push(project);
        }
    }

    projects.sort_by(|left, right| {
        left.relative_path
            .cmp(&right.relative_path)
            .then_with(|| left.name.cmp(&right.name))
    });
    for project in &mut projects {
        sort_diagnostics(&mut project.diagnostics);
    }
    let mut diagnostics = duplicate_name_diagnostics(&projects);
    sort_diagnostics(&mut diagnostics);
    let state = if projects
        .iter()
        .any(|project| project.state == ProjectState::Error)
    {
        WorkspaceState::Error
    } else {
        WorkspaceState::Open
    };

    Ok(WorkspaceSnapshot {
        protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
        config_anchor: workspace_primary.path,
        name: workspace.name,
        state,
        projects,
        diagnostics,
    })
}

fn optional_mount_layer(
    tree: Option<&FileTree>,
    mount_name: &str,
) -> Result<Option<Layer>, DiscoveryFailure> {
    tree.map(|tree| optional_layer(tree, &RelativePath::root(), mount_name))
        .transpose()
        .map(Option::flatten)
}

fn required_layer(
    tree: &FileTree,
    directory: &RelativePath,
    description: &str,
) -> Result<Layer, DiscoveryFailure> {
    optional_layer(tree, directory, description)?.ok_or_else(|| {
        failure(
            WORKSPACE_CONFIG_MISSING,
            format!("no Morphir configuration found at {description}"),
            Some(directory.clone()),
        )
    })
}

fn optional_layer(
    tree: &FileTree,
    directory: &RelativePath,
    description: &str,
) -> Result<Option<Layer>, DiscoveryFailure> {
    let candidates = found_primary_candidates(tree, directory);
    match candidates.as_slice() {
        [] => Ok(None),
        [path] => parse_layer(tree, path).map(Some),
        paths => Err(ambiguous_failure(description, paths)),
    }
}

fn optional_user_layer(
    tree: &FileTree,
    primary: &RelativePath,
) -> Result<Option<Layer>, DiscoveryFailure> {
    let candidates = found_adjacent_user_candidates(tree, primary);
    match candidates.as_slice() {
        [] => Ok(None),
        [path] => parse_layer(tree, path).map(Some),
        paths => Err(ambiguous_failure("workspace user override", paths)),
    }
}

fn parse_layer(tree: &FileTree, path: &RelativePath) -> Result<Layer, DiscoveryFailure> {
    let text = tree.file_text(path).expect("candidate must be a text file");
    let value = parse_config(path.as_str(), text).map_err(|error| {
        failure(
            WORKSPACE_CONFIG_INVALID,
            format!(
                "invalid Morphir configuration at `{path}`: {error}",
                path = path.as_str()
            ),
            Some(path.clone()),
        )
    })?;
    Ok(Layer {
        path: path.clone(),
        value,
    })
}

fn ambiguous_failure(description: &str, paths: &[RelativePath]) -> DiscoveryFailure {
    let listed = paths
        .iter()
        .map(|path| format!("`{}`", path.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    failure(
        WORKSPACE_CONFIG_AMBIGUOUS,
        format!("multiple Morphir configurations found for {description}: {listed}"),
        paths.first().cloned(),
    )
}

fn decode_workspace(
    effective: &Value,
    path: &RelativePath,
) -> Result<WorkspaceView, DiscoveryFailure> {
    match effective.get("workspace") {
        None => Ok(WorkspaceView::default()),
        Some(value) => {
            let workspace: WorkspaceView =
                serde_json::from_value(value.clone()).map_err(|error| {
                    failure(
                        WORKSPACE_CONFIG_INVALID,
                        format!("invalid workspace section at `{}`: {error}", path.as_str()),
                        Some(path.clone()),
                    )
                })?;
            if let Some(default_member) = &workspace.default_member {
                RelativePath::parse(default_member.clone()).map_err(|error| {
                    failure(
                        WORKSPACE_PATH_NOT_CONFINED,
                        error.to_string(),
                        Some(path.clone()),
                    )
                })?;
            }
            Ok(workspace)
        }
    }
}

fn decode_root_project(
    effective: &Value,
    anchor: &RelativePath,
) -> Result<ProjectSnapshot, DiscoveryFailure> {
    let project =
        decode_project(effective, &RelativePath::root()).map_err(|problem| match problem {
            ProjectDecodeError::Invalid(message) => failure(
                WORKSPACE_CONFIG_INVALID,
                format!("invalid root project at `{}`: {message}", anchor.as_str()),
                Some(anchor.clone()),
            ),
            ProjectDecodeError::NotConfined(message) => {
                failure(WORKSPACE_PATH_NOT_CONFINED, message, Some(anchor.clone()))
            }
        })?;
    Ok(ProjectSnapshot {
        name: project.name,
        version: project.version,
        relative_path: RelativePath::root(),
        config_anchor: Some(anchor.clone()),
        source_directory: project.source_directory,
        state: ProjectState::Unloaded,
        diagnostics: Vec::new(),
    })
}

#[allow(clippy::too_many_arguments)]
fn discover_member(
    tree: &FileTree,
    directory: &RelativePath,
    system: &Value,
    global: &Value,
    shared_workspace: &Value,
    shared_workspace_user: &Value,
    environment: &Value,
    cli_overlay: &Value,
    collector: &mut Option<&mut dyn EffectiveConfigCollector>,
) -> Option<ProjectSnapshot> {
    let primary_candidates = found_primary_candidates(tree, directory);
    let primary_path = match primary_candidates.as_slice() {
        [] => return None,
        [path] => path.clone(),
        paths => {
            let listed = paths
                .iter()
                .map(|path| format!("`{}`", path.as_str()))
                .collect::<Vec<_>>()
                .join(", ");
            return Some(error_project(
                directory,
                paths.first().cloned(),
                WORKSPACE_MEMBER_INVALID,
                format!("multiple member configurations found: {listed}"),
            ));
        }
    };
    let member_primary = match parse_member_layer(tree, &primary_path) {
        Ok(layer) => layer,
        Err(problem) => {
            return Some(error_project(
                directory,
                problem.path,
                WORKSPACE_MEMBER_INVALID,
                problem.message,
            ));
        }
    };
    let member_user = match member_user_layer(tree, &primary_path) {
        Ok(layer) => layer,
        Err(problem) => {
            return Some(error_project(
                directory,
                problem.path,
                WORKSPACE_MEMBER_INVALID,
                problem.message,
            ));
        }
    };
    let effective = member_effective_config(MemberConfigLayers {
        system,
        global,
        shared_workspace,
        member_primary: &member_primary.value,
        shared_workspace_user,
        member_user: member_user.as_ref().map(|layer| &layer.value),
        environment,
        cli_overlay,
    });
    let project = match decode_project(&effective, directory) {
        Ok(project) => project,
        Err(ProjectDecodeError::Invalid(message)) => {
            return Some(error_project(
                directory,
                Some(primary_path),
                WORKSPACE_MEMBER_INVALID,
                message,
            ));
        }
        Err(ProjectDecodeError::NotConfined(message)) => {
            return Some(error_project(
                directory,
                Some(primary_path),
                WORKSPACE_PATH_NOT_CONFINED,
                message,
            ));
        }
    };

    if let Some(collector) = collector.as_deref_mut() {
        collector.project(directory, &effective);
    }
    Some(ProjectSnapshot {
        name: project.name,
        version: project.version,
        relative_path: directory.clone(),
        config_anchor: Some(primary_path),
        source_directory: project.source_directory,
        state: ProjectState::Unloaded,
        diagnostics: Vec::new(),
    })
}

fn parse_member_layer(tree: &FileTree, path: &RelativePath) -> Result<Layer, MemberProblem> {
    let text = tree.file_text(path).expect("candidate must be a text file");
    let value = parse_config(path.as_str(), text).map_err(|error| MemberProblem {
        message: format!(
            "invalid member configuration at `{}`: {error}",
            path.as_str()
        ),
        path: Some(path.clone()),
    })?;
    Ok(Layer {
        path: path.clone(),
        value,
    })
}

fn member_user_layer(
    tree: &FileTree,
    primary: &RelativePath,
) -> Result<Option<Layer>, MemberProblem> {
    let candidates = found_adjacent_user_candidates(tree, primary);
    match candidates.as_slice() {
        [] => Ok(None),
        [path] => parse_member_layer(tree, path).map(Some),
        paths => Err(MemberProblem {
            message: format!(
                "multiple member user overrides found: {}",
                paths
                    .iter()
                    .map(|path| format!("`{}`", path.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            path: paths.first().cloned(),
        }),
    }
}

fn decode_project(
    effective: &Value,
    project_path: &RelativePath,
) -> Result<DecodedProject, ProjectDecodeError> {
    let value = effective
        .get("project")
        .ok_or_else(|| ProjectDecodeError::Invalid("missing project section".to_owned()))?;
    let project: ProjectView = serde_json::from_value(value.clone()).map_err(|error| {
        ProjectDecodeError::Invalid(format!("invalid project section: {error}"))
    })?;
    let source_directory = RelativePath::parse(project.source_directory)
        .map_err(|error| ProjectDecodeError::NotConfined(error.to_string()))?;
    project_path
        .join(source_directory.as_str())
        .map_err(|error| ProjectDecodeError::NotConfined(error.to_string()))?;
    Ok(DecodedProject {
        name: project.name,
        version: project.version,
        source_directory,
    })
}

struct DecodedProject {
    name: String,
    version: Option<String>,
    source_directory: RelativePath,
}

fn default_source_directory() -> String {
    "src".to_owned()
}

fn member_effective_config(layers: MemberConfigLayers<'_>) -> Value {
    let empty = Value::Object(Map::new());
    merge_all([
        &builtin_defaults(),
        layers.system,
        layers.global,
        layers.shared_workspace,
        layers.member_primary,
        layers.shared_workspace_user,
        layers.member_user.unwrap_or(&empty),
        layers.environment,
        layers.cli_overlay,
    ])
}

fn without_project_or_workspace(value: &Value) -> Value {
    let mut stripped = value.clone();
    if let Some(object) = stripped.as_object_mut() {
        object.remove("project");
        object.remove("workspace");
    }
    stripped
}

fn member_directories(
    tree: &FileTree,
    config_anchor: &RelativePath,
    members: &[String],
    excludes: &[String],
) -> Result<Vec<RelativePath>, DiscoveryFailure> {
    members
        .iter()
        .chain(excludes)
        .try_for_each(|pattern| validate_pattern(pattern, config_anchor))?;
    let member_matchers = compile_patterns(members, config_anchor)?;
    let exclude_matchers = compile_patterns(excludes, config_anchor)?;
    let matches = tree
        .directories()
        .filter(|directory| {
            member_matchers
                .iter()
                .any(|matcher| matcher.is_match(directory.as_str()))
        })
        .filter(|directory| {
            !exclude_matchers
                .iter()
                .any(|matcher| matcher.is_match(directory.as_str()))
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    Ok(matches.into_iter().collect())
}

fn compile_patterns(
    patterns: &[String],
    config_anchor: &RelativePath,
) -> Result<Vec<GlobMatcher>, DiscoveryFailure> {
    patterns
        .iter()
        .map(|pattern| {
            GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
                .map(|glob| glob.compile_matcher())
                .map_err(|error| {
                    failure(
                        WORKSPACE_MEMBER_INVALID,
                        format!("invalid workspace member pattern `{pattern}`: {error}"),
                        Some(config_anchor.clone()),
                    )
                })
        })
        .collect()
}

fn validate_pattern(pattern: &str, config_anchor: &RelativePath) -> Result<(), DiscoveryFailure> {
    let bytes = pattern.as_bytes();
    let windows_prefix = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if pattern.starts_with('/')
        || pattern.contains('\\')
        || windows_prefix
        || pattern.split('/').any(|component| component == "..")
    {
        return Err(failure(
            WORKSPACE_PATH_NOT_CONFINED,
            format!("workspace pattern `{pattern}` is not confined to the development root"),
            Some(config_anchor.clone()),
        ));
    }
    Ok(())
}

fn error_project(
    directory: &RelativePath,
    anchor: Option<RelativePath>,
    code: &str,
    message: String,
) -> ProjectSnapshot {
    let diagnostic = WorkspaceDiagnostic {
        severity: DiagnosticSeverity::Error,
        code: code.to_owned(),
        message,
        path: anchor.clone(),
        project_path: Some(directory.clone()),
    };
    ProjectSnapshot {
        name: directory.as_str().to_owned(),
        version: None,
        relative_path: directory.clone(),
        config_anchor: anchor,
        source_directory: RelativePath::parse("src").expect("default source path is confined"),
        state: ProjectState::Error,
        diagnostics: vec![diagnostic],
    }
}

fn duplicate_name_diagnostics(projects: &[ProjectSnapshot]) -> Vec<WorkspaceDiagnostic> {
    let mut names = BTreeMap::<&str, Vec<&RelativePath>>::new();
    for project in projects
        .iter()
        .filter(|project| project.state != ProjectState::Error)
    {
        names
            .entry(project.name.as_str())
            .or_default()
            .push(&project.relative_path);
    }
    names
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(name, mut paths)| {
            paths.sort();
            let listed = paths
                .iter()
                .map(|path| path.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            WorkspaceDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: WORKSPACE_MEMBER_DUPLICATE_NAME.to_owned(),
                message: format!("duplicate project name `{name}` at paths: {listed}"),
                path: None,
                project_path: None,
            }
        })
        .collect()
}

fn sort_diagnostics(diagnostics: &mut Vec<WorkspaceDiagnostic>) {
    diagnostics.sort_by(|left, right| {
        left.project_path
            .cmp(&right.project_path)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| severity_order(left.severity).cmp(&severity_order(right.severity)))
            .then_with(|| left.message.cmp(&right.message))
    });
    diagnostics.dedup();
}

const fn severity_order(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Info => 0,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Error => 2,
    }
}

fn failure(code: &str, message: String, path: Option<RelativePath>) -> DiscoveryFailure {
    DiscoveryFailure {
        code: code.to_owned(),
        message,
        path,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{
        EffectiveConfigCollector, MemberConfigLayers, discover_internal, member_effective_config,
        without_project_or_workspace,
    };
    use crate::{
        DiscoveryRequest, FileEntry, FileTree, RelativePath, WORKSPACE_DISCOVERY_PROTOCOL,
    };

    #[derive(Default)]
    struct CountingCollector {
        roots: usize,
        projects: Vec<RelativePath>,
    }

    impl EffectiveConfigCollector for CountingCollector {
        fn root(&mut self, _effective: &serde_json::Value) {
            self.roots += 1;
        }

        fn project(&mut self, path: &RelativePath, _effective: &serde_json::Value) {
            self.projects.push(path.clone());
        }
    }

    fn collection_request() -> DiscoveryRequest {
        DiscoveryRequest {
            protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
            development_root: FileTree {
                entries: BTreeMap::from([
                    (RelativePath::root(), FileEntry::Directory),
                    (
                        RelativePath::parse("morphir.toml").unwrap(),
                        FileEntry::File {
                            text: "[workspace]\nmembers = ['packages/*']\n[project]\nname = 'acme/root'\n"
                                .to_owned(),
                        },
                    ),
                    (
                        RelativePath::parse("packages/orders").unwrap(),
                        FileEntry::Directory,
                    ),
                    (
                        RelativePath::parse("packages/orders/morphir.toml").unwrap(),
                        FileEntry::File {
                            text: "[project]\nname = 'acme/orders'\n".to_owned(),
                        },
                    ),
                ]),
            },
            morphir_home: None,
            system_config: None,
            environment: BTreeMap::new(),
            cli_overlay: json!({}),
        }
    }

    #[test]
    fn effective_configs_are_collected_only_when_a_sink_is_supplied() {
        let request = collection_request();
        let ordinary = discover_internal(request.clone(), None).unwrap();
        let mut collector = CountingCollector::default();
        let detailed = discover_internal(request, Some(&mut collector)).unwrap();

        assert_eq!(ordinary, detailed);
        assert_eq!(collector.roots, 1);
        assert_eq!(
            collector.projects,
            [
                RelativePath::root(),
                RelativePath::parse("packages/orders").unwrap(),
            ]
        );
    }

    #[test]
    fn member_merge_inherits_only_shared_root_user_sections() {
        let empty = json!({});
        let root_user = json!({
            "workspace": { "name": "root-user-workspace" },
            "project": {
                "name": "root/user",
                "version": "2.0.0",
                "source_directory": "root-user-src"
            },
            "ir": { "strict_mode": true, "mode": "root-user" }
        });
        let shared_root_user = without_project_or_workspace(&root_user);
        let member_primary = json!({
            "project": {
                "name": "member/primary",
                "version": "1.0.0",
                "source_directory": "member-src"
            },
            "ir": { "format_version": 3, "mode": "member-primary" }
        });
        let member_user = json!({
            "project": { "version": "3.0.0" },
            "ir": { "mode": "member-user" }
        });

        let effective = member_effective_config(MemberConfigLayers {
            system: &empty,
            global: &empty,
            shared_workspace: &empty,
            member_primary: &member_primary,
            shared_workspace_user: &shared_root_user,
            member_user: Some(&member_user),
            environment: &empty,
            cli_overlay: &empty,
        });

        assert_eq!(effective["project"]["name"], "member/primary");
        assert_eq!(effective["project"]["version"], "3.0.0");
        assert_eq!(effective["project"]["source_directory"], "member-src");
        assert!(effective.get("workspace").is_none());
        assert_eq!(effective["ir"]["strict_mode"], true);
        assert_eq!(effective["ir"]["format_version"], 3);
        assert_eq!(effective["ir"]["mode"], "member-user");
    }
}

//! Canonical, filesystem-free workspace discovery.

mod decoding;
mod diagnostics;
mod identity;
mod layers;
mod members;
mod patterns;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use morphir_config::{builtin_defaults, merge_all};
use serde_json::{Map, Value};

use crate::{
    DiscoveryFailure, DiscoveryPurpose, DiscoveryRequest, DiscoveryResponse, FileEntry, FileTree,
    ProjectOrigin, ProjectSnapshot, ProjectSource, ProjectState, RelativePath, SourceSelection,
    WORKSPACE_CONFIG_INVALID, WORKSPACE_DISCOVERY_PROTOCOL, WORKSPACE_LANGUAGE_ID_EMPTY,
    WORKSPACE_PROJECT_NAME_EMPTY, WORKSPACE_PROTOCOL_UNSUPPORTED, WORKSPACE_SELECTION_DUPLICATE,
    WORKSPACE_SELECTION_EMPTY, WORKSPACE_SELECTION_INVALID, WORKSPACE_SELECTION_NAME_REQUIRED,
    WORKSPACE_SELECTION_OUTSIDE_ROOT, WORKSPACE_SYMLINK_UNSUPPORTED, WorkspaceDiscoveryDetails,
    WorkspaceSnapshot, WorkspaceState,
};
use decoding::{decode_root_project, decode_workspace};
use diagnostics::{duplicate_name_diagnostics, failure, sort_diagnostics};
use layers::{optional_user_layer, required_layer, shared_layers, without_project_or_workspace};
use members::discover_member;
use patterns::member_directories;

pub use identity::{SourceIdentity, discover_with_identity};

/// Discovers a Morphir workspace from portable, root-confined inputs.
///
/// The request contains only provider-neutral wire types, so a browser, native
/// host, or other provider can construct the same discovery input:
///
/// ```
/// use std::collections::BTreeMap;
///
/// use morphir_workspace::{
///     DiscoveryRequest, FileEntry, FileTree, RelativePath, WORKSPACE_DISCOVERY_PROTOCOL,
///     discover,
/// };
///
/// let development_root = FileTree {
///     entries: BTreeMap::from([
///         (RelativePath::root(), FileEntry::Directory),
///         (
///             RelativePath::parse("morphir.toml").expect("a confined wire path"),
///             FileEntry::File {
///                 text: "[project]\nname = 'acme/orders'\n".to_owned(),
///             },
///         ),
///     ]),
/// };
/// let request = DiscoveryRequest {
///     protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
///     development_root,
///     morphir_home: None,
///     system_config: None,
///     environment: BTreeMap::new(),
///     cli_overlay: serde_json::Value::Object(Default::default()),
///     purpose: Default::default(),
/// };
///
/// let snapshot = discover(request)
///     .into_result()
///     .expect("the portable request should describe a valid project");
/// assert_eq!(snapshot.projects.len(), 1);
/// assert_eq!(snapshot.projects[0].name, "acme/orders");
/// ```
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

pub(super) trait EffectiveConfigCollector {
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
    if !request.cli_overlay.is_null() && !request.cli_overlay.is_object() {
        return Err(failure(
            WORKSPACE_CONFIG_INVALID,
            "CLI overlay must be a JSON object or null".to_owned(),
            None,
        ));
    }
    reject_unmaterialized_symlinks(&request.development_root, "development root")?;
    if let Some(tree) = request.morphir_home.as_ref() {
        reject_unmaterialized_symlinks(tree, "Morphir Home")?;
    }
    if let Some(tree) = request.system_config.as_ref() {
        reject_unmaterialized_symlinks(tree, "system configuration")?;
    }

    if let DiscoveryPurpose::AdHocSources {
        project,
        sources,
        language_id,
    } = &request.purpose
    {
        return discover_ad_hoc_sources(&request, project, sources, language_id, collector);
    }

    let root = RelativePath::root();
    let workspace_primary = required_layer(&request.development_root, &root, "workspace root")?;
    let workspace_user = optional_user_layer(&request.development_root, &workspace_primary.path)?;
    let shared = shared_layers(&request)?;
    let empty = Value::Object(Map::new());
    let shared_workspace_user = workspace_user
        .as_ref()
        .map(|layer| without_project_or_workspace(&layer.value));
    let workspace_effective = merge_all([
        &builtin_defaults(),
        shared.system_value.as_ref().unwrap_or(&empty),
        shared.global_value.as_ref().unwrap_or(&empty),
        &workspace_primary.value,
        workspace_user
            .as_ref()
            .map(|layer| &layer.value)
            .unwrap_or(&empty),
        &shared.environment,
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
            shared.system_value.as_ref().unwrap_or(&empty),
            shared.global_value.as_ref().unwrap_or(&empty),
            &shared_workspace,
            shared_workspace_user.as_ref().unwrap_or(&empty),
            &shared.environment,
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
        config_anchor: Some(workspace_primary.path),
        name: workspace.name,
        state,
        projects,
        diagnostics,
    })
}

/// Discovers the single project an explicit source selection describes.
///
/// This is the [`DiscoveryPurpose::AdHocSources`] path: no workspace layer is
/// required and no member scan runs. The selection's root is carried verbatim
/// into [`ProjectSnapshot::relative_path`] and never recomputed from the
/// selected paths — see the module-level discussion of why that matters.
///
/// A [`ProjectSource::Manifest`] selection borrows the identity of a manifest
/// the host already resolved. The host owns configuration precedence, so it
/// states that identity as the overlay's `project.name`; this function only
/// confirms the manifest exists and records it as the project's origin. It
/// never reads the manifest's layers, which would make every provider
/// re-implement precedence the host has already applied.
///
/// Only language-neutral checks run here. Rules that depend on how a source
/// becomes a module — how many *distinct* sources a selection holds, whether
/// two of them collide, whether an explicit name satisfies the package
/// contract — belong to the provider; see [`discover_with_identity`].
fn discover_ad_hoc_sources(
    request: &DiscoveryRequest,
    project: &ProjectSource,
    sources: &SourceSelection,
    language_id: &str,
    collector: Option<&mut dyn EffectiveConfigCollector>,
) -> Result<WorkspaceSnapshot, DiscoveryFailure> {
    if language_id.is_empty() {
        return Err(failure(
            WORKSPACE_LANGUAGE_ID_EMPTY,
            format!(
                "ad-hoc selection rooted at `{}` has an empty language id",
                sources.root.as_str()
            ),
            Some(sources.root.clone()),
        ));
    }

    let name = resolve_synthesized_project_name(&request.cli_overlay, &sources.root)?;

    let origin = match project {
        ProjectSource::Synthesized => ProjectOrigin::Synthesized {
            inputs: sources.paths.clone(),
        },
        ProjectSource::Manifest { path } => {
            if name.is_none() {
                return Err(failure(
                    WORKSPACE_SELECTION_NAME_REQUIRED,
                    format!(
                        "ad-hoc selection borrowing manifest `{}` has no explicit name; the host states the manifest's project name as the overlay's `project.name`",
                        path.as_str()
                    ),
                    Some(path.clone()),
                ));
            }
            if !request.development_root.contains_file(path) {
                return Err(failure(
                    WORKSPACE_SELECTION_INVALID,
                    format!("manifest `{}` is not a file in the request", path.as_str()),
                    Some(path.clone()),
                ));
            }
            ProjectOrigin::Manifest { path: path.clone() }
        }
    };

    validate_ad_hoc_selection(&request.development_root, sources)?;

    let shared = shared_layers(request)?;
    let empty = Value::Object(Map::new());
    // An ad-hoc project reads no manifest layer: a synthesized one has none,
    // and a manifest-origin one had its manifest resolved by the host, which
    // passes the result through the overlay.
    let effective = merge_all([
        &builtin_defaults(),
        shared.system_value.as_ref().unwrap_or(&empty),
        shared.global_value.as_ref().unwrap_or(&empty),
        &shared.environment,
        &request.cli_overlay,
    ]);

    if let Some(collector) = collector {
        collector.root(&effective);
        collector.project(&sources.root, &effective);
    }

    let config_anchor = match &origin {
        ProjectOrigin::Manifest { path } => Some(path.clone()),
        ProjectOrigin::Synthesized { .. } => None,
    };
    let project = ProjectSnapshot {
        name: name.unwrap_or_default(),
        version: None,
        relative_path: sources.root.clone(),
        config_anchor,
        source_directory: RelativePath::root(),
        state: ProjectState::Unloaded,
        diagnostics: Vec::new(),
        origin,
        exposed_modules: None,
    };

    Ok(WorkspaceSnapshot {
        protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
        config_anchor: None,
        name: None,
        state: WorkspaceState::Open,
        projects: vec![project],
        diagnostics: Vec::new(),
    })
}

/// Reads and validates an explicit project name override from the CLI
/// overlay, at `project.name`.
///
/// Only an explicit overlay value counts as a supplied name. This reads
/// `request.cli_overlay` directly — never the effective configuration merged
/// from built-in defaults, shared system/global layers or the environment —
/// because merging any of those in would silently turn a default into a
/// project identity, which is worse than refusing to name the project at
/// all.
///
/// Policy for the value once present: it must be a JSON string, and it must
/// not be empty or whitespace-only. Whitespace-only is rejected rather than
/// silently treated as "no name was given", because an override this
/// visibly broken deserves a loud diagnostic naming the exact problem,
/// rather than silently falling through to the single-source cardinality
/// rule and failing (or not) for an unrelated reason.
///
/// A name that survives that check is stored *trimmed*, not as written.
/// Surrounding whitespace on a project identity is never meaningful, and
/// this value's origin is a command line — `morphir compile --package-name`
/// — where a stray space is a shell artefact, not something an author can
/// see and re-edit the way they can a line of manifest text. The CLI
/// already trims before it gets here, so storing the raw string would make
/// routing the flag through discovery *lose* trimming that shipped
/// behaviour has today, and would let `"acme/widgets "` and `"acme/widgets"`
/// name two different packages.
fn resolve_synthesized_project_name(
    cli_overlay: &Value,
    context: &RelativePath,
) -> Result<Option<String>, DiscoveryFailure> {
    let Some(name_value) = cli_overlay.pointer("/project/name") else {
        return Ok(None);
    };
    let Some(name) = name_value.as_str() else {
        return Err(failure(
            WORKSPACE_CONFIG_INVALID,
            format!("CLI overlay `project.name` must be a string, found `{name_value}`"),
            Some(context.clone()),
        ));
    };
    if name.trim().is_empty() {
        return Err(failure(
            WORKSPACE_PROJECT_NAME_EMPTY,
            "CLI overlay `project.name` must not be empty or whitespace-only".to_owned(),
            Some(context.clone()),
        ));
    }
    Ok(Some(name.trim().to_owned()))
}

/// Validates an ad-hoc source selection against the tree it selects from.
///
/// Checks run cheapest-first: an empty selection and repeated paths are
/// caller mistakes visible from the request alone, confinement is checked
/// against the selection's root and paths, and only then does the tree get
/// consulted to confirm every selected path names a file. How many *distinct*
/// sources the selection holds is not measured here: that depends on the
/// language, so the provider checks it (see [`discover_with_identity`]).
fn validate_ad_hoc_selection(
    tree: &FileTree,
    sources: &SourceSelection,
) -> Result<(), DiscoveryFailure> {
    if sources.paths.is_empty() {
        return Err(failure(
            WORKSPACE_SELECTION_EMPTY,
            format!(
                "ad-hoc selection rooted at `{}` selects no sources",
                sources.root.as_str()
            ),
            Some(sources.root.clone()),
        ));
    }

    let mut seen = BTreeSet::new();
    for path in &sources.paths {
        if !seen.insert(path) {
            return Err(failure(
                WORKSPACE_SELECTION_DUPLICATE,
                format!(
                    "selected path `{}` is repeated in the selection",
                    path.as_str()
                ),
                Some(path.clone()),
            ));
        }
    }

    let outside_root: Vec<RelativePath> = sources
        .paths
        .iter()
        .filter(|path| !path_is_under_root(&sources.root, path))
        .cloned()
        .collect();
    if let Some(first) = outside_root.first() {
        let listed = outside_root
            .iter()
            .map(|path| format!("`{}`", path.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(failure(
            WORKSPACE_SELECTION_OUTSIDE_ROOT,
            format!(
                "selected paths are not under selection root `{}`: {listed}",
                sources.root.as_str()
            ),
            Some(first.clone()),
        ));
    }

    let not_files: Vec<RelativePath> = sources
        .paths
        .iter()
        .filter(|path| !tree.contains_file(path))
        .cloned()
        .collect();
    if let Some(first) = not_files.first() {
        let listed = not_files
            .iter()
            .map(|path| format!("`{}`", path.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(failure(
            WORKSPACE_SELECTION_INVALID,
            format!("selected paths do not resolve to files: {listed}"),
            Some(first.clone()),
        ));
    }

    Ok(())
}

/// Returns whether `path` lies strictly beneath `root`, comparing canonical
/// path segments rather than string prefixes.
///
/// A string-prefix check would wrongly accept `models-other/file` under root
/// `models` (they share the text prefix `models` but are siblings, not
/// parent and child), and would wrongly reject everything under root `.`
/// (the mount root has no segments to match against).
fn path_is_under_root(root: &RelativePath, path: &RelativePath) -> bool {
    let root_segments = path_segments(root);
    let path_segments = path_segments(path);
    path_segments.len() > root_segments.len() && path_segments.starts_with(root_segments.as_slice())
}

fn path_segments(path: &RelativePath) -> Vec<&str> {
    if path.as_str() == "." {
        Vec::new()
    } else {
        path.as_str().split('/').collect()
    }
}

fn reject_unmaterialized_symlinks(tree: &FileTree, mount: &str) -> Result<(), DiscoveryFailure> {
    if let Some((path, FileEntry::Symlink { target })) = tree
        .entries
        .iter()
        .find(|(_, entry)| matches!(entry, FileEntry::Symlink { .. }))
    {
        return Err(failure(
            WORKSPACE_SYMLINK_UNSUPPORTED,
            format!(
                "unmaterialized symlink `{}` to `{}` in {mount}; protocol-v1 hosts must materialize confined symlink targets before discovery",
                path.as_str(),
                target.as_str()
            ),
            Some(path.clone()),
        ));
    }
    Ok(())
}

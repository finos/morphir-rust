//! Layered loading: merges every configuration source in precedence order and
//! resolves the workspace/project context around the result.

use super::discovery::{
    config_root, discover_config_at, discover_config_candidates, discover_morphir_dir,
    native_global_config_candidates, native_system_config_candidates, project_config_candidates,
    user_override_candidates,
};
use super::members::{
    expand_members, is_confined, is_member, members_select, resolves_inside,
    unconfined_target_warning, unconfined_warning,
};
use super::provenance::{ConfigOrigin, ProvenanceState};
use super::sources::{
    ConfigLoadOptions, ConfigSource, ConfigSourceKind, ConfigSourceStatus, EffectiveConfig,
    EnvSelection, SourceSelection,
};
use anyhow::{Context, Result};
#[cfg(test)]
use morphir_common::config::deep_merge;
use morphir_common::config::env::{env_config_value, process_env_config_value};
use morphir_common::config::load_config_value;
use morphir_common::config::model::{MorphirConfig, ProjectSection, WorkspaceSection};
pub use morphir_config::builtin_defaults;
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use std::path::{Path, PathBuf};

/// Configuration context containing loaded config and resolved paths
#[derive(Debug, Clone)]
pub struct ConfigContext {
    /// Effective configuration decoded into the typed model
    pub config: MorphirConfig,
    /// Effective configuration as the merged, serialization-independent value
    pub effective: Value,
    /// Configuration sources considered, from lowest to highest precedence
    pub sources: Vec<ConfigSource>,
    /// Path to the config file
    pub config_path: PathBuf,
    /// Path to `.morphir/` directory (canonical folder)
    pub morphir_dir: PathBuf,
    /// Workspace root if in workspace
    pub workspace_root: Option<PathBuf>,
    /// Project root if in project
    pub project_root: Option<PathBuf>,
    /// Current project if in workspace
    pub current_project: Option<ProjectSection>,
    /// Human-readable warnings about removed or renamed keys.
    pub warnings: Vec<String>,
}

fn resolve_file_source(
    kind: ConfigSourceKind,
    selection: &SourceSelection,
    candidates: impl FnOnce() -> Vec<PathBuf>,
) -> Result<ConfigSource> {
    match selection {
        SourceSelection::Skip => Ok(ConfigSource::skipped(kind)),
        SourceSelection::Explicit(path) => Ok(ConfigSource::loaded(kind, path.clone())),
        SourceSelection::Discover => {
            let candidates = candidates();
            Ok(match discover_config_candidates(&candidates)? {
                Some(path) => {
                    ConfigSource::new(kind, Some(path), candidates, ConfigSourceStatus::Loaded)
                }
                None => ConfigSource::not_found(kind, candidates),
            })
        }
    }
}

/// Which part of a workspace a configuration layer speaks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayerScope {
    /// The whole workspace, or a standalone project.
    Workspace,
    /// Only the selected workspace member: the member's own configuration, or
    /// a user override sitting beside it.
    Member,
}

fn merge_source(state: &mut ProvenanceState, source: &ConfigSource) -> Result<()> {
    merge_scoped_source(
        state,
        source,
        LayerScope::Workspace,
        &mut IgnoredMemberKeys::default(),
    )
}

/// Files whose member layer set a key only the workspace may set.
#[derive(Debug, Default)]
struct IgnoredMemberKeys {
    /// Files that set `workspace.out_dir` from inside a member.
    out_dir: Vec<PathBuf>,
}

/// Merge one loaded source, first dropping the keys a member layer may not set.
///
/// `workspace.out_dir` is the one such key today: the out root is a property of
/// the whole workspace, so a member that set it would relocate every other
/// member's output as well. It is dropped from the layer before the merge
/// rather than repaired in the merged value afterwards, so both the value and
/// its provenance stay honest and whatever the workspace itself set — in its
/// own configuration or in a user override beside it — simply stays in place.
///
/// The member scope covers the member's `morphir.toml` and the
/// `morphir.user.toml` next to it alike. Only the member's own configuration
/// used to be caught, because the check ran on the merged value's provenance
/// and a user override records the kind `UserOverride` no matter which
/// directory it came from, so an override beside a member relocated the whole
/// workspace's out root.
fn merge_scoped_source(
    state: &mut ProvenanceState,
    source: &ConfigSource,
    scope: LayerScope,
    ignored: &mut IgnoredMemberKeys,
) -> Result<()> {
    if let (ConfigSourceStatus::Loaded, Some(path)) = (source.status, &source.path) {
        let declaring_path = std::path::absolute(path).with_context(|| {
            format!(
                "Failed to stabilize {} configuration path: {}",
                source.kind.name(),
                path.display()
            )
        })?;
        let mut layer = load_config_value(&declaring_path).with_context(|| {
            format!(
                "Failed to load {} configuration: {}",
                source.kind.name(),
                path.display()
            )
        })?;
        if scope == LayerScope::Member && take_workspace_out_dir(&mut layer) {
            ignored.out_dir.push(declaring_path.clone());
        }
        state.merge(
            &layer,
            ConfigOrigin {
                kind: source.kind,
                path: Some(declaring_path),
            },
        );
    }
    Ok(())
}

/// Remove `workspace.out_dir` from a layer, reporting whether it was there.
fn take_workspace_out_dir(layer: &mut Value) -> bool {
    layer
        .get_mut("workspace")
        .and_then(Value::as_object_mut)
        .is_some_and(|workspace| workspace.remove("out_dir").is_some())
}

fn decode_config(value: &Value, what: &str) -> Result<MorphirConfig> {
    serde_json::from_value(value.clone())
        .with_context(|| format!("Failed to decode {what} Morphir config"))
}

fn env_source(state: &mut ProvenanceState, options: &ConfigLoadOptions) -> ConfigSource {
    let layer = match &options.env {
        EnvSelection::Skip => return ConfigSource::skipped(ConfigSourceKind::Environment),
        EnvSelection::Process => process_env_config_value(&options.env_prefix),
        EnvSelection::Explicit(vars) => env_config_value(
            &options.env_prefix,
            vars.iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        ),
    };
    let status = match layer.as_object() {
        Some(map) if !map.is_empty() => ConfigSourceStatus::Loaded,
        _ => ConfigSourceStatus::NotFound,
    };
    state.merge(
        &layer,
        ConfigOrigin {
            kind: ConfigSourceKind::Environment,
            path: None,
        },
    );
    ConfigSource::new(ConfigSourceKind::Environment, None, Vec::new(), status)
}

struct WorkspaceMemberConfig {
    root: PathBuf,
    config_path: PathBuf,
}

/// The workspace a configuration path belongs to, found by walking up.
struct EnclosingWorkspace {
    root: PathBuf,
    config_path: PathBuf,
}

/// Find the workspace that owns `member_root`, if any.
///
/// Walks up from `member_root`'s parent looking for a configuration that
/// declares `[workspace]` and whose `members` list selects `member_root`,
/// literally or through a wildcard, without an `exclude` pattern ruling it out
/// again. A configuration that is unreadable, ambiguous, or not a workspace is
/// skipped and the walk continues, so a stray file above the project can never
/// make loading fail. A `members` entry that leaves the workspace directory is
/// skipped the same way `expand_members` skips it, and adds a warning.
fn find_enclosing_workspace(
    member_root: &Path,
    warnings: &mut Vec<String>,
) -> Option<EnclosingWorkspace> {
    let mut current = member_root.parent();
    while let Some(directory) = current {
        if let Some(config_path) = discover_config_at(directory).ok().flatten()
            && let Some(workspace) = load_config_value(&config_path)
                .ok()
                .and_then(|value| decode_config(&value, "workspace").ok())
                .and_then(|config| config.workspace)
            && members_select(
                directory,
                &workspace.members,
                &workspace.exclude,
                member_root,
                warnings,
            )
        {
            return Some(EnclosingWorkspace {
                root: directory.to_path_buf(),
                config_path,
            });
        }
        current = directory.parent();
    }
    None
}

/// Choose which member of a workspace to merge when the workspace's own
/// configuration is the one being loaded.
///
/// `default_member` wins, unless it names the workspace root or a directory
/// `exclude` rules out. Otherwise the expanded member list decides, and a list
/// that expands to exactly one member selects it. A wider list has no member in
/// view, so none is merged; running inside a member directory is how a specific
/// member is selected, and that path is handled by [`find_enclosing_workspace`].
///
/// A `default_member` that leaves the workspace directory selects nothing and
/// adds a warning; so does any `members` entry that does the same. Without that
/// check `workspace_root.join("../outside")` would be accepted as a member and
/// its configuration merged from outside the workspace entirely.
///
/// The same goes for a member that is confined as text but is a symbolic link
/// to a directory outside the workspace: it is resolved on disk before it is
/// accepted, and refused with a warning if it lands elsewhere.
fn select_member(
    workspace_root: &Path,
    workspace: &WorkspaceSection,
    warnings: &mut Vec<String>,
) -> Option<PathBuf> {
    if let Some(member) = workspace.default_member.as_ref() {
        if !is_confined(member) {
            warnings.push(unconfined_warning(member));
            return None;
        }
        let member = workspace_root.join(member);
        if !is_member(workspace_root, &workspace.exclude, &member) {
            return None;
        }
        if !resolves_inside(workspace_root, &member) {
            warnings.push(unconfined_target_warning(&member));
            return None;
        }
        return Some(member);
    }
    match expand_members(
        workspace_root,
        &workspace.members,
        &workspace.exclude,
        warnings,
    )
    .as_slice()
    {
        [only] => Some(only.clone()),
        _ => None,
    }
}

/// Merge the selected workspace member's configuration, returning its root and primary path.
fn merge_workspace_member(
    state: &mut ProvenanceState,
    sources: &mut Vec<ConfigSource>,
    workspace_root: Option<&Path>,
    root_config: &MorphirConfig,
    warnings: &mut Vec<String>,
    ignored: &mut IgnoredMemberKeys,
) -> Result<Option<WorkspaceMemberConfig>> {
    let (Some(ws_root), Some(ws)) = (workspace_root, &root_config.workspace) else {
        return Ok(None);
    };
    let Some(member_path) = select_member(ws_root, ws, warnings) else {
        return Ok(None);
    };

    match discover_config_at(&member_path)? {
        Some(member_config) => {
            let source =
                ConfigSource::loaded(ConfigSourceKind::WorkspaceMember, member_config.clone());
            merge_scoped_source(state, &source, LayerScope::Member, ignored)?;
            sources.push(source);
            Ok(Some(WorkspaceMemberConfig {
                root: member_path,
                config_path: member_config,
            }))
        }
        None => {
            sources.push(ConfigSource::not_found(
                ConfigSourceKind::WorkspaceMember,
                project_config_candidates(&member_path).to_vec(),
            ));
            Ok(None)
        }
    }
}

/// Merge user overrides adjacent to the project primary path, then the member primary path.
fn merge_user_overrides(
    state: &mut ProvenanceState,
    sources: &mut Vec<ConfigSource>,
    options: &ConfigLoadOptions,
    project_config: Option<&Path>,
    member_config: Option<&Path>,
    ignored: &mut IgnoredMemberKeys,
) -> Result<()> {
    match &options.user_override {
        SourceSelection::Discover => {
            // An override beside the workspace primary speaks for the whole
            // workspace; one beside the member primary speaks only for that
            // member, and so may not set `workspace.out_dir`.
            let primary_paths = project_config
                .into_iter()
                .map(|path| (path, LayerScope::Workspace))
                .chain(
                    member_config
                        .filter(|member| Some(*member) != project_config)
                        .map(|path| (path, LayerScope::Member)),
                )
                .collect::<Vec<_>>();
            if primary_paths.is_empty() {
                sources.push(ConfigSource::skipped(ConfigSourceKind::UserOverride));
                return Ok(());
            }
            let mut found_layout = false;
            for (primary_path, scope) in primary_paths {
                let Some(candidates) = user_override_candidates(primary_path) else {
                    continue;
                };
                found_layout = true;
                let source = resolve_file_source(
                    ConfigSourceKind::UserOverride,
                    &SourceSelection::Discover,
                    || candidates.to_vec(),
                )?;
                merge_scoped_source(state, &source, scope, ignored)?;
                sources.push(source);
            }
            if !found_layout {
                sources.push(ConfigSource::skipped(ConfigSourceKind::UserOverride));
            }
        }
        selection => {
            let source = resolve_file_source(ConfigSourceKind::UserOverride, selection, Vec::new)?;
            merge_source(state, &source)?;
            sources.push(source);
        }
    }
    Ok(())
}

/// Compute the effective configuration from every configured source.
///
/// Sources are merged from lowest to highest precedence: built-in defaults,
/// system, global user, project, selected workspace member, user override(s),
/// and environment variables. `project_config` is the discovered or explicitly
/// selected project configuration; pass `None` to inspect the non-project
/// layers alone.
///
/// ```no_run
/// use morphir_devkit::{ConfigLoadOptions, discover_config, load_effective_config};
/// use std::path::Path;
///
/// # fn main() -> anyhow::Result<()> {
/// let project = discover_config(Path::new("."))?;
/// let effective = load_effective_config(project.as_deref(), &ConfigLoadOptions::default())?;
/// for source in &effective.sources {
///     println!("{:<16} {:<10} {}", source.kind.name(), source.status.label(), source.location());
/// }
/// # Ok(())
/// # }
/// ```
pub fn load_effective_config(
    project_config: Option<&Path>,
    options: &ConfigLoadOptions,
) -> Result<EffectiveConfig> {
    let mut state = ProvenanceState::default();
    let mut warnings = Vec::new();
    let mut ignored = IgnoredMemberKeys::default();
    state.merge(
        &builtin_defaults(),
        ConfigOrigin {
            kind: ConfigSourceKind::Defaults,
            path: None,
        },
    );
    let mut sources = vec![ConfigSource::new(
        ConfigSourceKind::Defaults,
        None,
        Vec::new(),
        ConfigSourceStatus::Loaded,
    )];

    let system = resolve_file_source(ConfigSourceKind::System, &options.system, || {
        native_system_config_candidates().to_vec()
    })?;
    merge_source(&mut state, &system)?;
    sources.push(system);

    let global = resolve_file_source(
        ConfigSourceKind::Global,
        &options.global,
        native_global_config_candidates,
    )?;
    merge_source(&mut state, &global)?;
    sources.push(global);

    // A configuration that lies inside a workspace member is the member layer,
    // not the project layer: the enclosing workspace configuration goes
    // underneath it so the two layers keep their documented precedence no
    // matter which of the two files the caller happened to discover.
    let selected_root = project_config.and_then(config_root).map(Path::to_path_buf);
    let enclosing = selected_root
        .as_deref()
        .and_then(|root| find_enclosing_workspace(root, &mut warnings));
    let project_layer = enclosing.as_ref().map_or_else(
        || project_config.map(Path::to_path_buf),
        |workspace| Some(workspace.config_path.clone()),
    );

    let project = match &project_layer {
        Some(path) => ConfigSource::loaded(ConfigSourceKind::Project, path.clone()),
        None => ConfigSource::not_found(ConfigSourceKind::Project, Vec::new()),
    };
    merge_source(&mut state, &project)?;
    sources.push(project);

    let root_config = decode_config(state.value(), "project")?;
    let (workspace_root, member_config) = match (enclosing, selected_root.clone(), project_config) {
        (Some(workspace), Some(member_root), Some(member_path)) => {
            let source =
                ConfigSource::loaded(ConfigSourceKind::WorkspaceMember, member_path.to_path_buf());
            merge_scoped_source(&mut state, &source, LayerScope::Member, &mut ignored)?;
            sources.push(source);
            (
                Some(workspace.root),
                Some(WorkspaceMemberConfig {
                    root: member_root,
                    config_path: member_path.to_path_buf(),
                }),
            )
        }
        _ => {
            let workspace_root = root_config
                .is_workspace()
                .then(|| selected_root.clone())
                .flatten();
            let member = merge_workspace_member(
                &mut state,
                &mut sources,
                workspace_root.as_deref(),
                &root_config,
                &mut warnings,
                &mut ignored,
            )?;
            (workspace_root, member)
        }
    };

    merge_user_overrides(
        &mut state,
        &mut sources,
        options,
        project_layer.as_deref(),
        member_config
            .as_ref()
            .map(|member| member.config_path.as_path()),
        &mut ignored,
    )?;

    sources.push(env_source(&mut state, options));

    let (value, provenance) = state.into_parts();

    Ok(EffectiveConfig {
        value,
        sources,
        workspace_root,
        member_root: member_config.map(|member| member.root),
        ignored_member_out_dir: ignored.out_dir,
        warnings,
        provenance,
    })
}

/// Load configuration and determine workspace/project context
pub fn load_config_context(config_path: &Path) -> Result<ConfigContext> {
    load_config_context_with(config_path, &ConfigLoadOptions::default())
}

/// Load configuration with an explicitly selected global source and no other layers.
pub fn load_config_context_with_global(
    config_path: &Path,
    global_config_path: Option<&Path>,
) -> Result<ConfigContext> {
    let options = ConfigLoadOptions {
        global: global_config_path
            .map(Path::to_path_buf)
            .map_or(SourceSelection::Skip, SourceSelection::Explicit),
        ..ConfigLoadOptions::project_only()
    };
    load_config_context_with(config_path, &options)
}

/// Warnings for keys that were removed or renamed. `ir.mode` is still applied
/// as an alias for `ir.layout` when `ir.layout` was not itself set explicitly;
/// the other keys are ignored.
///
/// Whether `ir.layout` was set explicitly cannot be decided from `effective`
/// alone: the merged value always carries the built-in default
/// (`ir.layout = "single-file"`) even when no source set it, so a bare
/// pointer check can never see the difference. Callers that need that
/// distinction (see [`load_config_context_with`]) resolve it from the
/// configuration provenance instead and append the extra warning themselves.
pub fn deprecated_key_warnings(effective: &Value) -> Vec<String> {
    let mut warnings = Vec::new();
    if effective.pointer("/project/output_directory").is_some() {
        warnings.push(
            "project.output_directory was removed; all task output lives under workspace.out_dir"
                .to_owned(),
        );
    }
    if effective.pointer("/workspace/output_dir").is_some() {
        warnings.push("workspace.output_dir was renamed to workspace.out_dir".to_owned());
    }
    if effective.pointer("/ir/mode").is_some() {
        warnings.push(
            "ir.mode is deprecated; use ir.layout = \"single-file\" or \"document-tree\""
                .to_owned(),
        );
    }
    warnings
}

/// Load configuration from every selected source and determine workspace/project context
pub fn load_config_context_with(
    config_path: &Path,
    options: &ConfigLoadOptions,
) -> Result<ConfigContext> {
    let config_dir = config_root(config_path)
        .ok_or_else(|| anyhow::anyhow!("Config file has no parent directory"))?;

    let effective_config = load_effective_config(Some(config_path), options)?;
    // `ir.layout` always has an entry in the merged value because the
    // built-in defaults set it; only the provenance can tell whether a real
    // source (as opposed to the defaults layer) set it explicitly.
    let layout_explicit = effective_config
        .origin_for_key("ir.layout")
        .is_some_and(|origin| origin.kind != ConfigSourceKind::Defaults);
    let EffectiveConfig {
        value: effective,
        sources,
        workspace_root,
        member_root,
        ignored_member_out_dir,
        warnings: member_warnings,
        ..
    } = effective_config;
    let config = decode_config(&effective, "merged")
        .with_context(|| format!("Failed to load Morphir config: {}", config_path.display()))?;

    let mut warnings = member_warnings;
    warnings.extend(deprecated_key_warnings(&effective));
    if effective.pointer("/ir/mode").is_some() && layout_explicit {
        warnings.push("ir.mode is ignored because ir.layout is set explicitly".to_owned());
    }
    for path in ignored_member_out_dir {
        warnings.push(format!(
            "workspace.out_dir is ignored in {}, which belongs to one workspace member; \
             the out root is shared by the whole workspace, so set it in the workspace configuration",
            path.display()
        ));
    }
    let mut config = config;
    if let Some(ir) = config.ir.as_mut()
        && let Some(mode) = ir.mode.take()
        && !layout_explicit
    {
        ir.layout = match mode.as_str() {
            "vfs" => "document-tree".to_owned(),
            _ => "single-file".to_owned(),
        };
    }

    // Inside a workspace the project root is the selected member, if any;
    // otherwise the configuration directory is the project root.
    let project_root = if workspace_root.is_some() {
        member_root
    } else {
        Some(config_dir.to_path_buf())
    };

    // Find or create .morphir/ directory
    let morphir_dir = discover_morphir_dir(config_dir).unwrap_or_else(|| {
        // Use project root if available, otherwise config dir
        project_root
            .as_ref()
            .map_or(config_dir, |v| v.as_path())
            .join(".morphir")
    });

    Ok(ConfigContext {
        current_project: config.project.clone(),
        config,
        effective,
        sources,
        config_path: config_path.to_path_buf(),
        morphir_dir,
        workspace_root,
        project_root,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::paths::resolve_path_relative_to_config;
    use morphir_common::config::ExposeSecret;
    use morphir_common::config::env::DEFAULT_ENV_PREFIX;
    use morphir_common::config::model::{CodegenSection, FrontendSection, IrSection};
    use serde::Serialize;
    use serde::de::DeserializeOwned;
    use std::process::Command;

    fn write_file(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().expect("config parent")).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn write_project_config(path: &Path) {
        write_file(path, "project:\n  name: Acme.Project\n  version: 1.0.0\n");
    }

    fn env(vars: &[(&str, &str)]) -> EnvSelection {
        EnvSelection::Explicit(
            vars.iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        )
    }

    /// No workspace-member configuration contributed to this context. A
    /// member that is selected but missing records a not-found source; one
    /// that is never selected records no source at all.
    fn no_member_was_merged(context: &ConfigContext) -> bool {
        context.project_root.is_none()
            && !context.sources.iter().any(|source| {
                source.kind == ConfigSourceKind::WorkspaceMember
                    && source.status == ConfigSourceStatus::Loaded
            })
    }

    fn source(sources: &[ConfigSource], kind: ConfigSourceKind) -> &ConfigSource {
        sources
            .iter()
            .find(|source| source.kind == kind)
            .unwrap_or_else(|| panic!("missing {kind:?} source"))
    }

    /// Expected effective value: the given layers on top of the built-in defaults.
    fn with_defaults(value: Value) -> Value {
        deep_merge(&builtin_defaults(), &value)
    }

    fn model_defaults<T: DeserializeOwned + Serialize>(seed: &Value) -> (Value, Value) {
        let from_seed: T = serde_json::from_value(seed.clone()).expect("seed decodes");
        let from_empty: T = serde_json::from_value(json!({})).expect("empty decodes");
        (
            serde_json::to_value(from_seed).unwrap(),
            serde_json::to_value(from_empty).unwrap(),
        )
    }

    fn run_isolated_cwd_helper(test_name: &str, declaring_dir: &Path, changed_dir: &Path) {
        let output = Command::new(std::env::current_exe().unwrap())
            .arg(test_name)
            .arg("--exact")
            .arg("--ignored")
            .arg("--nocapture")
            .current_dir(declaring_dir)
            .env("MORPHIR_TEST_CHANGED_CWD", changed_dir)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "isolated working-directory regression helper failed"
        );
    }

    #[test]
    fn builtin_defaults_match_the_typed_model() {
        let defaults = builtin_defaults();
        let (seeded, empty) = model_defaults::<FrontendSection>(&defaults["frontend"]);
        assert_eq!(seeded, empty);
        let (seeded, empty) = model_defaults::<IrSection>(&defaults["ir"]);
        assert_eq!(seeded, empty);
        let (seeded, empty) = model_defaults::<CodegenSection>(&defaults["codegen"]);
        assert_eq!(seeded, empty);

        let config: MorphirConfig = serde_json::from_value(defaults).unwrap();
        assert!(!config.is_project());
        assert!(!config.is_workspace());
    }

    #[test]
    fn resolves_hidden_config_paths_from_the_project_root() {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join(".morphir").join("morphir.yaml");
        write_project_config(&config_path);

        let context = load_config_context_with_global(&config_path, None).unwrap();

        assert_eq!(context.project_root.as_deref(), Some(root.path()));
        assert_eq!(context.morphir_dir, root.path().join(".morphir"));
        assert_eq!(
            resolve_path_relative_to_config(Path::new("src"), &config_path),
            root.path().join("src")
        );
    }

    #[test]
    fn merges_yaml_global_config_below_toml_project_config() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global").join("morphir.yaml");
        let project = root.path().join("project").join("morphir.toml");
        write_file(
            &global,
            "frontend:\n  language: elm\nir:\n  strict_mode: true\n",
        );
        write_file(
            &project,
            "[project]\nname = \"Acme.Project\"\nversion = \"1.0.0\"\n\n[ir]\nstrict_mode = false\n",
        );

        let context = load_config_context_with_global(&project, Some(&global)).unwrap();

        assert_eq!(
            context.config.frontend.unwrap().language.as_deref(),
            Some("elm")
        );
        assert!(!context.config.ir.unwrap().strict_mode);
    }

    #[test]
    fn merges_every_layer_in_precedence_order() {
        let root = tempfile::tempdir().unwrap();
        let system = root.path().join("etc").join("morphir").join("morphir.toml");
        let global = root
            .path()
            .join("home")
            .join(".morphir")
            .join("morphir.yaml");
        let project = root.path().join("project").join("morphir.yaml");
        let user = root.path().join("project").join("morphir.user.toml");
        write_file(
            &system,
            "[logging]\nlevel = \"warn\"\nformat = \"json\"\n\n[ui]\ncolor = false\n\n[cache]\nenabled = false\n",
        );
        write_file(&global, "logging:\n  level: info\nui:\n  theme: dark\n");
        write_file(
            &project,
            "project:\n  name: Acme.Project\n  version: 1.0.0\nlogging:\n  level: debug\ncodegen:\n  targets: [go, scala]\n",
        );
        write_file(
            &user,
            "[logging]\noutput = \"debug.log\"\n\n[codegen]\ntargets = [\"typescript\"]\n",
        );

        let options = ConfigLoadOptions {
            system: SourceSelection::Explicit(system.clone()),
            global: SourceSelection::Explicit(global.clone()),
            user_override: SourceSelection::Discover,
            env: env(&[
                ("MORPHIR_LOGGING__LEVEL", "error"),
                ("MORPHIR_IR__STRICT_MODE", "true"),
                ("HOME", "/home/alice"),
            ]),
            env_prefix: DEFAULT_ENV_PREFIX.to_string(),
        };
        let context = load_config_context_with(&project, &options).unwrap();

        assert_eq!(
            context.effective,
            with_defaults(json!({
                "project": {"name": "Acme.Project", "version": "1.0.0"},
                "logging": {"level": "error", "format": "json", "output": "debug.log"},
                "ui": {"color": false, "theme": "dark"},
                "cache": {"enabled": false},
                "codegen": {"targets": ["typescript"]},
                "ir": {"strict_mode": true},
            }))
        );
        assert!(context.config.ir.unwrap().strict_mode);
        assert_eq!(context.config.codegen.unwrap().targets, vec!["typescript"]);

        let kinds = context
            .sources
            .iter()
            .map(|source| (source.kind, source.status))
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                (ConfigSourceKind::Defaults, ConfigSourceStatus::Loaded),
                (ConfigSourceKind::System, ConfigSourceStatus::Loaded),
                (ConfigSourceKind::Global, ConfigSourceStatus::Loaded),
                (ConfigSourceKind::Project, ConfigSourceStatus::Loaded),
                (ConfigSourceKind::UserOverride, ConfigSourceStatus::Loaded),
                (ConfigSourceKind::Environment, ConfigSourceStatus::Loaded),
            ]
        );
        assert_eq!(
            source(&context.sources, ConfigSourceKind::UserOverride).path,
            Some(user)
        );
        assert!(
            context
                .sources
                .windows(2)
                .all(|pair| pair[0].priority < pair[1].priority)
        );
    }

    #[test]
    fn reports_missing_layers_without_failing() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("morphir.toml");
        write_file(
            &project,
            "[project]\nname = \"Acme.Project\"\nversion = \"1\"\n",
        );

        let options = ConfigLoadOptions {
            system: SourceSelection::Skip,
            global: SourceSelection::Skip,
            user_override: SourceSelection::Discover,
            env: env(&[]),
            env_prefix: DEFAULT_ENV_PREFIX.to_string(),
        };
        let context = load_config_context_with(&project, &options).unwrap();

        assert_eq!(
            source(&context.sources, ConfigSourceKind::System).status,
            ConfigSourceStatus::Skipped
        );
        let user = source(&context.sources, ConfigSourceKind::UserOverride);
        assert_eq!(user.status, ConfigSourceStatus::NotFound);
        assert_eq!(
            user.candidates,
            user_override_candidates(&project).expect("standard layout")
        );
        assert_eq!(
            source(&context.sources, ConfigSourceKind::Environment).status,
            ConfigSourceStatus::NotFound
        );
        assert_eq!(context.config.project.unwrap().name, "Acme.Project");
        // Built-in defaults are visible in the effective value.
        assert_eq!(context.effective["ir"]["format_version"], json!(4));
    }

    /// Version 4 is the default, but a project pinning version 3 must keep it
    /// through the whole merge chain — the defaults layer below it and a global
    /// layer that says 4 above it. This is what keeps v3 usable while v4 moves.
    #[test]
    fn a_project_can_pin_ir_format_version_3() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global").join("morphir.toml");
        let project = root.path().join("project").join("morphir.toml");
        write_file(&global, "[ir]\nformat_version = 4\n");
        write_file(
            &project,
            "[project]\nname = \"Legacy.Project\"\nversion = \"1\"\n\n[ir]\nformat_version = 3\nlayout = \"single-file\"\n",
        );

        let context = load_config_context_with_global(&project, Some(&global)).unwrap();

        let ir = context.config.ir.expect("ir section");
        assert_eq!(ir.format_version, 3);
        assert_eq!(ir.layout, "single-file");
        assert_eq!(context.effective["ir"]["format_version"], json!(3));
        assert_eq!(
            context
                .effective
                .get("ir")
                .and_then(|ir| ir.get("strict_mode")),
            Some(&json!(false)),
            "the defaults layer still supplies the settings the project left alone"
        );
    }

    /// The environment layer sits above every file, so it can move a project
    /// off its pinned version — deliberately, and only when asked.
    #[test]
    fn the_environment_can_override_a_pinned_ir_format_version() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("morphir.toml");
        write_file(&project, "[ir]\nformat_version = 3\n");

        let options = ConfigLoadOptions {
            env: env(&[("MORPHIR_IR__FORMAT_VERSION", "4")]),
            ..ConfigLoadOptions::project_only()
        };
        let context = load_config_context_with(&project, &options).unwrap();

        assert_eq!(context.config.ir.unwrap().format_version, 4);
        assert_eq!(context.effective["ir"]["format_version"], json!(4));
    }

    #[test]
    fn legacy_json_project_does_not_clobber_lower_layers_with_nulls() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global").join("morphir.toml");
        let project = root.path().join("project").join("morphir.json");
        write_file(&global, "[frontend]\nlanguage = \"elm\"\n");
        write_file(
            &project,
            r#"{"name": "Legacy.Project", "sourceDirectory": "src", "exposedModules": []}"#,
        );

        let context = load_config_context_with_global(&project, Some(&global)).unwrap();

        assert_eq!(
            context.config.frontend.unwrap().language.as_deref(),
            Some("elm")
        );
        assert_eq!(context.config.project.unwrap().name, "Legacy.Project");
    }

    #[test]
    fn loads_non_project_layers_without_a_project() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("morphir.yaml");
        write_file(&global, "ui:\n  theme: dark\n");

        let options = ConfigLoadOptions {
            system: SourceSelection::Skip,
            global: SourceSelection::Explicit(global),
            user_override: SourceSelection::Discover,
            env: env(&[("MORPHIR_UI__COLOR", "false")]),
            env_prefix: DEFAULT_ENV_PREFIX.to_string(),
        };
        let effective = load_effective_config(None, &options).unwrap();

        assert_eq!(
            effective.value,
            with_defaults(json!({"ui": {"theme": "dark", "color": false}}))
        );
        assert_eq!(
            source(&effective.sources, ConfigSourceKind::Project).status,
            ConfigSourceStatus::NotFound
        );
        assert_eq!(
            source(&effective.sources, ConfigSourceKind::UserOverride).status,
            ConfigSourceStatus::Skipped
        );
        assert_eq!(
            effective
                .sources
                .iter()
                .filter(|source| source.kind == ConfigSourceKind::UserOverride)
                .count(),
            1,
            "a missing project records one skipped user-override source"
        );
        assert!(effective.workspace_root.is_none());
    }

    #[test]
    fn workspace_member_layers_sit_between_project_and_user_override() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("morphir.toml");
        let member = root
            .path()
            .join("packages")
            .join("orders")
            .join("morphir.yaml");
        let workspace_user = root.path().join("morphir.user.yaml");
        let member_user = root
            .path()
            .join("packages")
            .join("orders")
            .join("morphir.user.yaml");
        write_file(
            &workspace,
            "[workspace]\nmembers = [\"packages/orders\"]\n\n[ir]\nformat_version = 3\nlayout = \"single-file\"\nstrict_mode = false\n",
        );
        write_file(
            &member,
            "project:\n  name: acme/orders\n  version: 1.0.0\nir:\n  strict_mode: true\n",
        );
        write_file(&workspace_user, "ir:\n  layout: document-tree\n");
        write_file(&member_user, "ir:\n  format_version: 4\n");

        let options = ConfigLoadOptions {
            user_override: SourceSelection::Discover,
            ..ConfigLoadOptions::project_only()
        };
        let context = load_config_context_with(&workspace, &options).unwrap();

        assert_eq!(context.workspace_root.as_deref(), Some(root.path()));
        assert_eq!(
            context.project_root,
            Some(root.path().join("packages").join("orders"))
        );
        assert_eq!(context.current_project.unwrap().name, "acme/orders");
        let ir = context.config.ir.unwrap();
        assert!(ir.strict_mode);
        assert_eq!(ir.layout, "document-tree");
        assert_eq!(ir.format_version, 4);

        let kinds = context
            .sources
            .iter()
            .filter(|source| source.status == ConfigSourceStatus::Loaded)
            .map(|source| source.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                ConfigSourceKind::Defaults,
                ConfigSourceKind::Project,
                ConfigSourceKind::WorkspaceMember,
                ConfigSourceKind::UserOverride,
                ConfigSourceKind::UserOverride,
            ]
        );
    }

    #[test]
    fn tracks_origins_through_workspace_user_and_environment_layers() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("morphir.toml");
        let member = root
            .path()
            .join("packages")
            .join("orders")
            .join("morphir.yaml");
        let user_path = root.path().join("morphir.user.toml");
        write_file(
            &workspace,
            "[workspace]\nmembers = [\"packages/orders\"]\n\n[registry]\nendpoint = \"https://project\"\n",
        );
        write_file(
            &member,
            "project:\n  name: acme/orders\n  version: 1.0.0\nregistry:\n  member_only: true\n",
        );
        write_file(
            &user_path,
            "[registry]\ntoken = { env = \"REGISTRY_TOKEN\" }\n",
        );

        let effective = load_effective_config(
            Some(&workspace),
            &ConfigLoadOptions {
                user_override: SourceSelection::Explicit(user_path.clone()),
                env: env(&[("MORPHIR_REGISTRY__TIMEOUT", "30")]),
                ..ConfigLoadOptions::project_only()
            },
        )
        .unwrap();

        assert_eq!(
            effective
                .origin_for_key("registry.token")
                .unwrap()
                .path
                .as_deref(),
            Some(user_path.as_path())
        );
        assert_eq!(
            effective.origin_for_key("registry.endpoint").unwrap().kind,
            ConfigSourceKind::Project
        );
        assert_eq!(
            effective.origin_for_key("registry.timeout").unwrap().kind,
            ConfigSourceKind::Environment
        );
    }

    #[test]
    fn loaded_relative_file_origin_survives_a_working_directory_change() {
        let root = tempfile::tempdir().unwrap();
        let declaring_dir = root.path().join("declaring");
        let changed_dir = root.path().join("changed");
        write_file(
            &declaring_dir.join("morphir.toml"),
            "[registry]\ntoken = { file = \"secrets/token\" }\n",
        );
        write_file(&declaring_dir.join("secrets/token"), "declaring-file-token");
        write_file(&changed_dir.join("secrets/token"), "changed-file-token");

        run_isolated_cwd_helper(
            "config::loader::tests::resolve_relative_file_after_cwd_change_helper",
            &declaring_dir,
            &changed_dir,
        );
    }

    #[test]
    #[ignore]
    fn resolve_relative_file_after_cwd_change_helper() {
        let changed_dir = PathBuf::from(std::env::var_os("MORPHIR_TEST_CHANGED_CWD").unwrap());
        let effective = load_effective_config(
            Some(Path::new("morphir.toml")),
            &ConfigLoadOptions::project_only(),
        )
        .unwrap();

        std::env::set_current_dir(changed_dir).unwrap();
        let secret = effective.resolve_secret("registry.token").unwrap();

        assert!(
            secret.expose_secret() == "declaring-file-token",
            "relative file resolution changed after the process working directory changed"
        );
    }

    #[test]
    fn loaded_relative_command_origin_survives_a_working_directory_change() {
        let root = tempfile::tempdir().unwrap();
        let declaring_dir = root.path().join("declaring");
        let changed_dir = root.path().join("changed");
        let helper_name = format!("secret-helper{}", std::env::consts::EXE_SUFFIX);
        let command = format!(
            "[registry]\ntoken = {{ command = [\"./{helper_name}\", \"config::loader::tests::relative_command_writes_marker_helper\", \"--exact\", \"--ignored\", \"--nocapture\"] }}\n"
        );
        write_file(&declaring_dir.join("morphir.toml"), &command);
        std::fs::create_dir_all(&changed_dir).unwrap();
        crate::config::test_support::install_helper_executable(&declaring_dir.join(&helper_name));
        crate::config::test_support::install_helper_executable(&changed_dir.join(&helper_name));

        run_isolated_cwd_helper(
            "config::loader::tests::resolve_relative_command_after_cwd_change_helper",
            &declaring_dir,
            &changed_dir,
        );

        assert!(declaring_dir.join("command-marker").is_file());
        assert!(!changed_dir.join("command-marker").exists());
    }

    #[test]
    #[ignore]
    fn resolve_relative_command_after_cwd_change_helper() {
        let changed_dir = PathBuf::from(std::env::var_os("MORPHIR_TEST_CHANGED_CWD").unwrap());
        let effective = load_effective_config(
            Some(Path::new("morphir.toml")),
            &ConfigLoadOptions::project_only(),
        )
        .unwrap();

        std::env::set_current_dir(changed_dir).unwrap();
        assert!(effective.resolve_secret("registry.token").is_ok());
    }

    #[test]
    #[ignore]
    fn relative_command_writes_marker_helper() {
        std::fs::write("command-marker", b"executed").unwrap();
    }

    #[test]
    fn workspace_layouts_merge_adjacent_overrides_in_precedence_order() {
        let root = tempfile::tempdir().unwrap();
        let layouts = [
            (
                "root",
                PathBuf::from("morphir.toml"),
                PathBuf::from("packages/orders/morphir.yaml"),
                PathBuf::from("morphir.user.toml"),
                PathBuf::from("packages/orders/morphir.user.yaml"),
            ),
            (
                "morphir directory",
                PathBuf::from(".morphir/morphir.toml"),
                PathBuf::from("packages/orders/.morphir/morphir.yaml"),
                PathBuf::from(".morphir/morphir.user.toml"),
                PathBuf::from("packages/orders/.morphir/morphir.user.yaml"),
            ),
            (
                "dot config directory",
                PathBuf::from(".config/morphir/config.toml"),
                PathBuf::from("packages/orders/.config/morphir/config.yaml"),
                PathBuf::from(".config/morphir/config.user.toml"),
                PathBuf::from("packages/orders/.config/morphir/config.user.yaml"),
            ),
        ];

        for (name, workspace_primary, member_primary, workspace_user, member_user) in layouts {
            let case_root = root.path().join(name.replace(' ', "-"));
            let workspace = case_root.join(workspace_primary);
            let member = case_root.join(member_primary);
            let workspace_user = case_root.join(workspace_user);
            let member_user = case_root.join(member_user);
            write_file(
                &workspace,
                "[workspace]\nmembers = [\"packages/orders\"]\n\n[ir]\nmode = \"classic\"\n",
            );
            write_file(
                &member,
                "project:\n  name: acme/orders\n  version: 1.0.0\nir:\n  strict_mode: true\n",
            );
            write_file(&workspace_user, "[ir]\nformat_version = 3\n");
            write_file(&member_user, "ir:\n  format_version: 4\n");

            let context = load_config_context_with(
                &workspace,
                &ConfigLoadOptions {
                    user_override: SourceSelection::Discover,
                    ..ConfigLoadOptions::project_only()
                },
            )
            .unwrap();

            assert_eq!(
                context
                    .sources
                    .iter()
                    .filter(|source| source.status == ConfigSourceStatus::Loaded)
                    .map(|source| source.kind)
                    .collect::<Vec<_>>(),
                vec![
                    ConfigSourceKind::Defaults,
                    ConfigSourceKind::Project,
                    ConfigSourceKind::WorkspaceMember,
                    ConfigSourceKind::UserOverride,
                    ConfigSourceKind::UserOverride,
                ],
                "{name} layout"
            );
            assert_eq!(
                context.config.ir.expect("ir section").format_version,
                4,
                "the member override must win for the {name} layout"
            );
        }
    }

    #[test]
    fn missing_workspace_member_is_reported_not_fatal() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("morphir.toml");
        write_file(
            &workspace,
            "[workspace]\nmembers = [\"packages/missing\"]\n",
        );

        let context =
            load_config_context_with(&workspace, &ConfigLoadOptions::project_only()).unwrap();

        assert_eq!(context.workspace_root.as_deref(), Some(root.path()));
        assert!(context.project_root.is_none());
        let member = source(&context.sources, ConfigSourceKind::WorkspaceMember);
        assert_eq!(member.status, ConfigSourceStatus::NotFound);
        assert_eq!(member.candidates.len(), 6);
    }

    #[test]
    fn removed_and_renamed_keys_produce_warnings() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("morphir.toml");
        write_file(
            &config,
            "[project]\nname = \"acme/app\"\nversion = \"1.0.0\"\noutput_directory = \"old\"\n\n[workspace]\noutput_dir = \"old\"\n\n[ir]\nmode = \"vfs\"\n",
        );
        let context = load_config_context(&config).unwrap();
        assert_eq!(context.warnings.len(), 3, "{:?}", context.warnings);
        assert!(context.warnings[0].contains("project.output_directory"));
        assert!(context.warnings[1].contains("workspace.output_dir"));
        assert!(context.warnings[2].contains("ir.mode"));
        assert_eq!(context.config.ir.unwrap().layout, "document-tree");
    }

    #[test]
    fn clean_configs_have_no_warnings() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("morphir.toml");
        write_file(
            &config,
            "[project]\nname = \"acme/app\"\nversion = \"1.0.0\"\n",
        );
        assert!(load_config_context(&config).unwrap().warnings.is_empty());
    }

    #[test]
    fn explicit_layout_beats_deprecated_mode() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("morphir.toml");
        write_file(
            &config,
            "[project]\nname = \"acme/app\"\nversion = \"1.0.0\"\n\n[ir]\nmode = \"classic\"\nlayout = \"document-tree\"\n",
        );
        let context = load_config_context(&config).unwrap();
        assert_eq!(context.warnings.len(), 2, "{:?}", context.warnings);
        assert!(context.warnings[0].contains("ir.mode"));
        assert!(
            context.warnings[1].contains("ir.mode is ignored because ir.layout is set explicitly")
        );
        assert_eq!(context.config.ir.unwrap().layout, "document-tree");
    }

    #[test]
    fn mode_alone_maps_vfs_to_document_tree() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("morphir.toml");
        write_file(
            &config,
            "[project]\nname = \"acme/app\"\nversion = \"1.0.0\"\n\n[ir]\nmode = \"vfs\"\n",
        );
        let context = load_config_context(&config).unwrap();
        assert_eq!(context.warnings.len(), 1, "{:?}", context.warnings);
        assert!(context.warnings[0].contains("ir.mode"));
        assert_eq!(context.config.ir.unwrap().layout, "document-tree");
    }

    #[test]
    fn glob_member_patterns_expand_to_member_directories() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("morphir.toml");
        write_file(&workspace, "[workspace]\nmembers = [\"packages/*\"]\n");
        write_file(
            &root
                .path()
                .join("packages")
                .join("orders")
                .join("morphir.toml"),
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
        );

        let context =
            load_config_context_with(&workspace, &ConfigLoadOptions::project_only()).unwrap();

        assert_eq!(context.workspace_root.as_deref(), Some(root.path()));
        assert_eq!(
            context.project_root,
            Some(root.path().join("packages").join("orders"))
        );
        assert_eq!(context.current_project.unwrap().name, "acme/orders");
    }

    #[test]
    fn a_glob_that_expands_to_several_members_selects_none_without_a_default() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("morphir.toml");
        write_file(&workspace, "[workspace]\nmembers = [\"packages/*\"]\n");
        for name in ["orders", "billing"] {
            write_file(
                &root.path().join("packages").join(name).join("morphir.toml"),
                &format!("[project]\nname = \"acme/{name}\"\nversion = \"1.0.0\"\n"),
            );
        }

        let context =
            load_config_context_with(&workspace, &ConfigLoadOptions::project_only()).unwrap();

        assert_eq!(context.workspace_root.as_deref(), Some(root.path()));
        assert!(context.project_root.is_none());
        assert!(context.current_project.is_none());
    }

    #[test]
    fn default_member_selects_out_of_a_glob_expansion() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("morphir.toml");
        write_file(
            &workspace,
            "[workspace]\nmembers = [\"packages/*\"]\ndefault_member = \"packages/billing\"\n",
        );
        for name in ["orders", "billing"] {
            write_file(
                &root.path().join("packages").join(name).join("morphir.toml"),
                &format!("[project]\nname = \"acme/{name}\"\nversion = \"1.0.0\"\n"),
            );
        }

        let context =
            load_config_context_with(&workspace, &ConfigLoadOptions::project_only()).unwrap();

        assert_eq!(
            context.project_root,
            Some(root.path().join("packages").join("billing"))
        );
        assert_eq!(context.current_project.unwrap().name, "acme/billing");
    }

    #[test]
    fn a_member_configuration_resolves_its_enclosing_workspace() {
        let root = tempfile::tempdir().unwrap();
        write_file(
            &root.path().join("morphir.toml"),
            "[workspace]\nmembers = [\"packages/*\"]\n\n[ir]\nformat_version = 3\n",
        );
        let member = root
            .path()
            .join("packages")
            .join("orders")
            .join("morphir.toml");
        write_file(
            &member,
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n\n[ir]\nstrict_mode = true\n",
        );

        let context =
            load_config_context_with(&member, &ConfigLoadOptions::project_only()).unwrap();

        assert_eq!(context.workspace_root.as_deref(), Some(root.path()));
        assert_eq!(
            context.project_root,
            Some(root.path().join("packages").join("orders"))
        );
        assert_eq!(context.config_path, member);
        // The workspace configuration merged underneath the member's, so both
        // layers contribute and the member wins where they overlap.
        let ir = context.config.ir.unwrap();
        assert_eq!(ir.format_version, 3);
        assert!(ir.strict_mode);
        let kinds = context
            .sources
            .iter()
            .filter(|source| source.status == ConfigSourceStatus::Loaded)
            .map(|source| source.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                ConfigSourceKind::Defaults,
                ConfigSourceKind::Project,
                ConfigSourceKind::WorkspaceMember,
            ]
        );
        assert_eq!(
            source(&context.sources, ConfigSourceKind::Project)
                .path
                .as_deref(),
            Some(root.path().join("morphir.toml").as_path())
        );
    }

    #[test]
    fn a_project_an_unrelated_workspace_does_not_list_stays_standalone() {
        let root = tempfile::tempdir().unwrap();
        write_file(
            &root.path().join("morphir.toml"),
            "[workspace]\nmembers = [\"packages/*\"]\n",
        );
        let outsider = root.path().join("tools").join("cli").join("morphir.toml");
        write_file(
            &outsider,
            "[project]\nname = \"acme/cli\"\nversion = \"1.0.0\"\n",
        );

        let context =
            load_config_context_with(&outsider, &ConfigLoadOptions::project_only()).unwrap();

        assert!(context.workspace_root.is_none());
        assert_eq!(
            context.project_root,
            Some(root.path().join("tools").join("cli"))
        );
    }

    #[test]
    fn a_member_configuration_cannot_relocate_the_workspace_out_root() {
        let root = tempfile::tempdir().unwrap();
        write_file(
            &root.path().join("morphir.toml"),
            "[workspace]\nmembers = [\"packages/*\"]\nout_dir = \"build/out\"\n",
        );
        let member = root
            .path()
            .join("packages")
            .join("orders")
            .join("morphir.toml");
        write_file(
            &member,
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n\n[workspace]\nout_dir = \"member-out\"\n",
        );

        let context =
            load_config_context_with(&member, &ConfigLoadOptions::project_only()).unwrap();

        assert_eq!(context.config.workspace.unwrap().out_dir, "build/out");
        assert_eq!(context.warnings.len(), 1, "{:?}", context.warnings);
        assert!(
            context.warnings[0].contains("workspace.out_dir is ignored"),
            "{:?}",
            context.warnings
        );
    }

    #[test]
    fn an_ignored_member_out_dir_falls_back_to_the_default() {
        let root = tempfile::tempdir().unwrap();
        write_file(
            &root.path().join("morphir.toml"),
            "[workspace]\nmembers = [\"packages/*\"]\n",
        );
        let member = root
            .path()
            .join("packages")
            .join("orders")
            .join("morphir.toml");
        write_file(
            &member,
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n\n[workspace]\nout_dir = \"member-out\"\n",
        );

        let context =
            load_config_context_with(&member, &ConfigLoadOptions::project_only()).unwrap();

        assert_eq!(
            context.config.workspace.as_ref().unwrap().out_dir,
            ".morphir/out"
        );
        assert_eq!(
            crate::out::resolve_out_root(None, None, Some(&context), Path::new("/scratch")),
            root.path().join(".morphir").join("out")
        );
    }

    /// A one-member workspace with a `morphir.user.toml` beside the workspace
    /// primary, beside the member primary, or both. Returns the workspace
    /// primary path, which is what the tests load.
    fn workspace_with_user_overrides(
        root: &Path,
        workspace_user: Option<&str>,
        member_user: Option<&str>,
    ) -> PathBuf {
        let workspace = root.join("morphir.toml");
        write_file(&workspace, "[workspace]\nmembers = [\"packages/orders\"]\n");
        let member = root.join("packages").join("orders");
        write_file(
            &member.join("morphir.toml"),
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
        );
        if let Some(body) = workspace_user {
            write_file(&root.join("morphir.user.toml"), body);
        }
        if let Some(body) = member_user {
            write_file(&member.join("morphir.user.toml"), body);
        }
        workspace
    }

    fn with_user_overrides() -> ConfigLoadOptions {
        ConfigLoadOptions {
            user_override: SourceSelection::Discover,
            ..ConfigLoadOptions::project_only()
        }
    }

    #[test]
    fn a_user_override_beside_a_member_cannot_relocate_the_workspace_out_root() {
        // The member's own `morphir.toml` was already refused, but a
        // `morphir.user.toml` next to it records the source kind
        // `UserOverride`, so the old provenance check let it through and one
        // member's personal override moved every member's output.
        let root = tempfile::tempdir().unwrap();
        let workspace = workspace_with_user_overrides(
            root.path(),
            None,
            Some("[workspace]\nout_dir = \"member-out\"\n"),
        );

        let context = load_config_context_with(&workspace, &with_user_overrides()).unwrap();

        assert_eq!(
            context.config.workspace.as_ref().unwrap().out_dir,
            ".morphir/out"
        );
        assert_eq!(context.warnings.len(), 1, "{:?}", context.warnings);
        assert!(
            context.warnings[0].contains("workspace.out_dir is ignored"),
            "{:?}",
            context.warnings
        );
        assert!(
            context.warnings[0].contains("morphir.user.toml"),
            "the warning names the file it came from: {:?}",
            context.warnings
        );
    }

    #[test]
    fn a_user_override_beside_the_workspace_may_still_set_the_out_root() {
        let root = tempfile::tempdir().unwrap();
        let workspace = workspace_with_user_overrides(
            root.path(),
            Some("[workspace]\nout_dir = \"my-out\"\n"),
            None,
        );

        let context = load_config_context_with(&workspace, &with_user_overrides()).unwrap();

        assert!(context.warnings.is_empty(), "{:?}", context.warnings);
        assert_eq!(context.config.workspace.as_ref().unwrap().out_dir, "my-out");
    }

    #[test]
    fn a_member_override_never_beats_the_workspace_override_for_the_out_root() {
        // Both overrides set it. The member's is dropped from its layer, so
        // the workspace's survives rather than falling back to the default.
        let root = tempfile::tempdir().unwrap();
        let workspace = workspace_with_user_overrides(
            root.path(),
            Some("[workspace]\nout_dir = \"my-out\"\n"),
            Some("[workspace]\nout_dir = \"member-out\"\n"),
        );

        let context = load_config_context_with(&workspace, &with_user_overrides()).unwrap();

        assert_eq!(context.config.workspace.as_ref().unwrap().out_dir, "my-out");
        assert_eq!(context.warnings.len(), 1, "{:?}", context.warnings);
    }

    #[test]
    fn the_workspace_layer_may_still_set_the_out_root() {
        let root = tempfile::tempdir().unwrap();
        write_file(
            &root.path().join("morphir.toml"),
            "[workspace]\nmembers = [\"packages/*\"]\nout_dir = \"build/out\"\n",
        );
        let member = root
            .path()
            .join("packages")
            .join("orders")
            .join("morphir.toml");
        write_file(
            &member,
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
        );

        let context =
            load_config_context_with(&member, &ConfigLoadOptions::project_only()).unwrap();

        assert!(context.warnings.is_empty(), "{:?}", context.warnings);
        assert_eq!(context.config.workspace.unwrap().out_dir, "build/out");
    }

    /// The repository's own monorepo fixture excludes `packages/ignored` from
    /// a `packages/*` members list. `morphir-workspace` already refuses to
    /// treat that directory as a member; the loader must agree, or two crates
    /// read one configuration file two ways and the excluded project's output
    /// would nest under the workspace out root.
    #[test]
    fn an_excluded_directory_is_not_a_workspace_member() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/workspace-discovery/valid-monorepo");
        let options = ConfigLoadOptions::project_only();

        let ignored = load_config_context_with(
            &fixture
                .join("packages")
                .join("ignored")
                .join("morphir.toml"),
            &options,
        )
        .unwrap();
        assert_eq!(ignored.workspace_root, None);
        assert_eq!(
            ignored.project_root,
            Some(fixture.join("packages").join("ignored"))
        );

        // A sibling the exclude list does not name still joins the workspace.
        let orders = load_config_context_with(
            &fixture.join("packages").join("orders").join("morphir.yaml"),
            &options,
        )
        .unwrap();
        assert_eq!(orders.workspace_root.as_deref(), Some(fixture.as_path()));
        assert_eq!(
            orders.project_root,
            Some(fixture.join("packages").join("orders"))
        );
    }

    #[test]
    fn an_excluded_directory_is_never_the_selected_member() {
        let root = tempfile::tempdir().unwrap();
        write_file(
            &root.path().join("morphir.toml"),
            "[workspace]\nmembers = [\"packages/*\"]\nexclude = [\"packages/ignored\"]\n",
        );
        for name in ["orders", "ignored"] {
            write_file(
                &root.path().join("packages").join(name).join("morphir.toml"),
                &format!("[project]\nname = \"acme/{name}\"\nversion = \"1.0.0\"\n"),
            );
        }

        // Two directories match `packages/*`, but one is excluded, so exactly
        // one member remains and it is selected.
        let context = load_config_context_with(
            &root.path().join("morphir.toml"),
            &ConfigLoadOptions::project_only(),
        )
        .unwrap();
        assert_eq!(
            context.project_root,
            Some(root.path().join("packages").join("orders"))
        );
        assert_eq!(context.current_project.unwrap().name, "acme/orders");
    }

    /// `members = ["**"]` selects the workspace root itself. Merging the
    /// workspace configuration a second time as the member layer would
    /// re-attribute every value it sets to that layer, so a workspace that set
    /// `workspace.out_dir` would be told its own key is ignored.
    #[test]
    fn a_workspace_is_never_its_own_member() {
        let root = tempfile::tempdir().unwrap();
        write_file(
            &root.path().join("morphir.toml"),
            "[workspace]\nmembers = [\"**\"]\nout_dir = \"build/out\"\n\n[project]\nname = \"acme/root\"\nversion = \"1.0.0\"\n",
        );

        let context = load_config_context_with(
            &root.path().join("morphir.toml"),
            &ConfigLoadOptions::project_only(),
        )
        .unwrap();

        assert!(context.warnings.is_empty(), "{:?}", context.warnings);
        assert_eq!(
            context.config.workspace.as_ref().unwrap().out_dir,
            "build/out"
        );
        assert!(no_member_was_merged(&context));
    }

    /// A workspace laid out with a sibling directory the workspace does not
    /// own, so a member entry that escapes has somewhere real to land.
    fn workspace_beside_an_outsider(root: &Path, workspace_body: &str) -> PathBuf {
        write_file(
            &root.join("outside").join("morphir.toml"),
            "[project]\nname = \"acme/outside\"\nversion = \"1.0.0\"\n",
        );
        let workspace = root.join("workspace").join("morphir.toml");
        write_file(&workspace, workspace_body);
        workspace
    }

    #[test]
    fn a_default_member_that_leaves_the_workspace_is_ignored_with_a_warning() {
        let root = tempfile::tempdir().unwrap();
        let workspace = workspace_beside_an_outsider(
            root.path(),
            "[workspace]\nmembers = [\"packages/*\"]\ndefault_member = \"../outside\"\n",
        );

        let context =
            load_config_context_with(&workspace, &ConfigLoadOptions::project_only()).unwrap();

        assert!(
            no_member_was_merged(&context),
            "a member outside the workspace must never be merged: {:?}",
            context.sources
        );
        assert!(context.current_project.is_none());
        assert_eq!(context.warnings.len(), 1, "{:?}", context.warnings);
        assert!(
            context.warnings[0].contains("../outside") && context.warnings[0].contains("confined"),
            "{:?}",
            context.warnings
        );
    }

    #[test]
    fn an_absolute_member_entry_is_ignored_with_a_warning() {
        let root = tempfile::tempdir().unwrap();
        let absolute = root.path().join("outside").to_string_lossy().into_owned();
        let workspace = workspace_beside_an_outsider(
            root.path(),
            &format!(
                "[workspace]\nmembers = [\"{}\"]\n",
                absolute.replace('\\', "\\\\")
            ),
        );

        let context =
            load_config_context_with(&workspace, &ConfigLoadOptions::project_only()).unwrap();

        assert!(no_member_was_merged(&context), "{:?}", context.sources);
        assert_eq!(context.warnings.len(), 1, "{:?}", context.warnings);
        assert!(
            context.warnings[0].contains(&absolute),
            "{:?}",
            context.warnings
        );
    }

    #[test]
    fn a_backslash_separated_member_entry_is_ignored_with_a_warning() {
        // A backslash is a path separator on Windows, so `..\outside` escapes
        // there even though it is one odd directory name on Unix. The entry is
        // refused on every platform, so a workspace behaves the same way
        // wherever it is loaded.
        let root = tempfile::tempdir().unwrap();
        let workspace = workspace_beside_an_outsider(
            root.path(),
            "[workspace]\nmembers = [\"..\\\\outside\"]\n",
        );

        let context =
            load_config_context_with(&workspace, &ConfigLoadOptions::project_only()).unwrap();

        assert!(no_member_was_merged(&context), "{:?}", context.sources);
        assert_eq!(context.warnings.len(), 1, "{:?}", context.warnings);
        assert!(
            context.warnings[0].contains(r"..\outside"),
            "{:?}",
            context.warnings
        );
    }

    #[test]
    fn a_default_member_naming_the_root_selects_nothing() {
        let root = tempfile::tempdir().unwrap();
        write_file(
            &root.path().join("morphir.toml"),
            "[workspace]\nmembers = [\"packages/*\"]\ndefault_member = \".\"\n\n[project]\nname = \"acme/root\"\nversion = \"1.0.0\"\n",
        );

        let context = load_config_context_with(
            &root.path().join("morphir.toml"),
            &ConfigLoadOptions::project_only(),
        )
        .unwrap();

        assert!(no_member_was_merged(&context));
    }
}

//! Layered loading: merges every configuration source in precedence order and
//! resolves the workspace/project context around the result.

use super::discovery::{
    config_root, discover_config_at, discover_config_candidates, discover_morphir_dir,
    native_global_config_candidates, native_system_config_candidates, project_config_candidates,
    user_override_candidates,
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
use morphir_common::config::model::{MorphirConfig, ProjectSection};
use serde_json::{Value, json};
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
}

/// Built-in defaults: the lowest-precedence configuration layer.
///
/// These are the defaults the typed configuration model applies to the
/// `frontend`, `ir`, and `codegen` sections. Sections whose presence carries
/// meaning (`project`, `workspace`) are not seeded; their field defaults apply
/// only once a source declares the section.
pub fn builtin_defaults() -> Value {
    json!({
        "frontend": {
            "emit_parse_stage": true,
            "emit_parse_stage_fatal": false,
        },
        "ir": {
            "format_version": 4,
            "mode": "vfs",
            "strict_mode": false,
        },
        "codegen": {
            "targets": [],
            "output_format": "pretty",
        },
    })
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

fn merge_source(state: &mut ProvenanceState, source: &ConfigSource) -> Result<()> {
    if let (ConfigSourceStatus::Loaded, Some(path)) = (source.status, &source.path) {
        let declaring_path = std::path::absolute(path).with_context(|| {
            format!(
                "Failed to stabilize {} configuration path: {}",
                source.kind.name(),
                path.display()
            )
        })?;
        let layer = load_config_value(&declaring_path).with_context(|| {
            format!(
                "Failed to load {} configuration: {}",
                source.kind.name(),
                path.display()
            )
        })?;
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

/// Merge the selected workspace member's configuration, returning its root and primary path.
fn merge_workspace_member(
    state: &mut ProvenanceState,
    sources: &mut Vec<ConfigSource>,
    workspace_root: Option<&Path>,
    root_config: &MorphirConfig,
) -> Result<Option<WorkspaceMemberConfig>> {
    let (Some(ws_root), Some(ws)) = (workspace_root, &root_config.workspace) else {
        return Ok(None);
    };
    // Resolve the default member path (glob patterns are treated literally for now)
    let Some(member) = ws.default_member.as_ref().or_else(|| ws.members.first()) else {
        return Ok(None);
    };

    let member_path = ws_root.join(member);
    match discover_config_at(&member_path)? {
        Some(member_config) => {
            let source =
                ConfigSource::loaded(ConfigSourceKind::WorkspaceMember, member_config.clone());
            merge_source(state, &source)?;
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
) -> Result<()> {
    match &options.user_override {
        SourceSelection::Discover => {
            let primary_paths = project_config
                .into_iter()
                .chain(member_config.filter(|member| Some(*member) != project_config))
                .collect::<Vec<_>>();
            if primary_paths.is_empty() {
                sources.push(ConfigSource::skipped(ConfigSourceKind::UserOverride));
                return Ok(());
            }
            let mut found_layout = false;
            for primary_path in primary_paths {
                let Some(candidates) = user_override_candidates(primary_path) else {
                    continue;
                };
                found_layout = true;
                let source = resolve_file_source(
                    ConfigSourceKind::UserOverride,
                    &SourceSelection::Discover,
                    || candidates.to_vec(),
                )?;
                merge_source(state, &source)?;
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

    let project = match project_config {
        Some(path) => ConfigSource::loaded(ConfigSourceKind::Project, path.to_path_buf()),
        None => ConfigSource::not_found(ConfigSourceKind::Project, Vec::new()),
    };
    merge_source(&mut state, &project)?;
    sources.push(project);

    let project_root = project_config.and_then(config_root).map(Path::to_path_buf);
    let root_config = decode_config(state.value(), "project")?;
    let workspace_root = root_config
        .is_workspace()
        .then(|| project_root.clone())
        .flatten();

    let member_config = merge_workspace_member(
        &mut state,
        &mut sources,
        workspace_root.as_deref(),
        &root_config,
    )?;

    merge_user_overrides(
        &mut state,
        &mut sources,
        options,
        project_config,
        member_config
            .as_ref()
            .map(|member| member.config_path.as_path()),
    )?;

    sources.push(env_source(&mut state, options));

    let (value, provenance) = state.into_parts();

    Ok(EffectiveConfig {
        value,
        sources,
        workspace_root,
        member_root: member_config.map(|member| member.root),
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

/// Load configuration from every selected source and determine workspace/project context
pub fn load_config_context_with(
    config_path: &Path,
    options: &ConfigLoadOptions,
) -> Result<ConfigContext> {
    let config_dir = config_root(config_path)
        .ok_or_else(|| anyhow::anyhow!("Config file has no parent directory"))?;

    let EffectiveConfig {
        value: effective,
        sources,
        workspace_root,
        member_root,
        ..
    } = load_effective_config(Some(config_path), options)?;
    let config = decode_config(&effective, "merged")
        .with_context(|| format!("Failed to load Morphir config: {}", config_path.display()))?;

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
            "[project]\nname = \"Legacy.Project\"\nversion = \"1\"\n\n[ir]\nformat_version = 3\nmode = \"classic\"\n",
        );

        let context = load_config_context_with_global(&project, Some(&global)).unwrap();

        let ir = context.config.ir.expect("ir section");
        assert_eq!(ir.format_version, 3);
        assert_eq!(ir.mode, "classic");
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
            "[workspace]\nmembers = [\"packages/orders\"]\n\n[ir]\nformat_version = 3\nmode = \"classic\"\nstrict_mode = false\n",
        );
        write_file(
            &member,
            "project:\n  name: acme/orders\n  version: 1.0.0\nir:\n  strict_mode: true\n",
        );
        write_file(&workspace_user, "ir:\n  mode: vfs\n");
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
        assert_eq!(ir.mode, "vfs");
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
        std::fs::copy(
            std::env::current_exe().unwrap(),
            declaring_dir.join(&helper_name),
        )
        .unwrap();
        std::fs::copy(
            std::env::current_exe().unwrap(),
            changed_dir.join(&helper_name),
        )
        .unwrap();

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
}

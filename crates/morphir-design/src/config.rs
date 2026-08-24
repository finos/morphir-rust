use anyhow::{Context, Result, bail};
use morphir_common::config::deep_merge;
use morphir_common::config::env::{DEFAULT_ENV_PREFIX, env_config_value, process_env_config_value};
use morphir_common::config::load_config_value;
use morphir_common::config::model::{MorphirConfig, ProjectSection};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
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

/// Operating-system path conventions used for global user configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigPlatform {
    /// Linux and other systems that follow the XDG Base Directory specification.
    Xdg,
    /// macOS application-support path conventions, with XDG override support.
    MacOs,
    /// Windows Known Folder path conventions.
    Windows,
}

/// Kind of configuration source, ordered from lowest to highest precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigSourceKind {
    /// Built-in defaults compiled into the tool.
    Defaults,
    /// System-wide configuration (`/etc/morphir` or `%PROGRAMDATA%\morphir`).
    System,
    /// Global user configuration.
    Global,
    /// Project or workspace configuration.
    Project,
    /// Configuration of the selected workspace member.
    WorkspaceMember,
    /// Personal override stored in a project's `.morphir` directory.
    UserOverride,
    /// `MORPHIR_*` environment variables.
    Environment,
}

impl ConfigSourceKind {
    /// Numeric precedence; higher values override lower values.
    pub const fn priority(self) -> u32 {
        match self {
            Self::Defaults => 0,
            Self::System => 100,
            Self::Global => 200,
            Self::Project => 300,
            Self::WorkspaceMember => 350,
            Self::UserOverride => 400,
            Self::Environment => 600,
        }
    }

    /// Short name used in reports.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Defaults => "defaults",
            Self::System => "system",
            Self::Global => "global",
            Self::Project => "project",
            Self::WorkspaceMember => "workspace-member",
            Self::UserOverride => "user",
            Self::Environment => "env",
        }
    }
}

/// Whether a configuration source contributed to the effective configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigSourceStatus {
    /// The source existed and was merged.
    Loaded,
    /// The source was looked for but does not exist.
    NotFound,
    /// The caller asked to skip this source.
    Skipped,
}

impl ConfigSourceStatus {
    /// Human-readable status label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Loaded => "loaded",
            Self::NotFound => "not found",
            Self::Skipped => "skipped",
        }
    }
}

/// One configuration source considered while computing the effective configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSource {
    /// Source kind.
    pub kind: ConfigSourceKind,
    /// Numeric precedence of the source.
    pub priority: u32,
    /// Path that was loaded, when the source is a file.
    pub path: Option<PathBuf>,
    /// Every location that was checked for this source.
    pub candidates: Vec<PathBuf>,
    /// Outcome for this source.
    pub status: ConfigSourceStatus,
}

impl ConfigSource {
    fn new(
        kind: ConfigSourceKind,
        path: Option<PathBuf>,
        candidates: Vec<PathBuf>,
        status: ConfigSourceStatus,
    ) -> Self {
        Self {
            kind,
            priority: kind.priority(),
            path,
            candidates,
            status,
        }
    }

    fn loaded(kind: ConfigSourceKind, path: PathBuf) -> Self {
        Self::new(
            kind,
            Some(path.clone()),
            vec![path],
            ConfigSourceStatus::Loaded,
        )
    }

    fn not_found(kind: ConfigSourceKind, candidates: Vec<PathBuf>) -> Self {
        Self::new(kind, None, candidates, ConfigSourceStatus::NotFound)
    }

    fn skipped(kind: ConfigSourceKind) -> Self {
        Self::new(kind, None, Vec::new(), ConfigSourceStatus::Skipped)
    }

    /// Display string for the source location.
    pub fn location(&self) -> String {
        match (self.kind, &self.path) {
            (ConfigSourceKind::Defaults, _) => "(built-in)".to_string(),
            (ConfigSourceKind::Environment, _) => format!("{DEFAULT_ENV_PREFIX}_*"),
            (_, Some(path)) => path.display().to_string(),
            (_, None) => self
                .candidates
                .iter()
                .map(|candidate| candidate.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

/// How to select a file-based configuration source.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SourceSelection {
    /// Look the file up at its standard locations.
    #[default]
    Discover,
    /// Use exactly this file.
    Explicit(PathBuf),
    /// Do not consult the source.
    Skip,
}

/// How to select the environment-variable configuration source.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EnvSelection {
    /// Read the variables of the current process.
    #[default]
    Process,
    /// Use exactly these variables (useful for tests and tooling).
    Explicit(Vec<(String, String)>),
    /// Do not consult environment variables.
    Skip,
}

/// Options controlling which configuration sources are merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigLoadOptions {
    /// System configuration selection.
    pub system: SourceSelection,
    /// Global user configuration selection.
    pub global: SourceSelection,
    /// User override selection. `Discover` looks in the project root and, when
    /// a workspace member is selected, in the member root as well.
    pub user_override: SourceSelection,
    /// Environment variable selection.
    pub env: EnvSelection,
    /// Environment variable prefix without the trailing underscore.
    pub env_prefix: String,
}

impl Default for ConfigLoadOptions {
    fn default() -> Self {
        Self {
            system: SourceSelection::Discover,
            global: SourceSelection::Discover,
            user_override: SourceSelection::Discover,
            env: EnvSelection::Process,
            env_prefix: DEFAULT_ENV_PREFIX.to_string(),
        }
    }
}

impl ConfigLoadOptions {
    /// Consult only the project (and workspace member) configuration files.
    pub fn project_only() -> Self {
        Self {
            system: SourceSelection::Skip,
            global: SourceSelection::Skip,
            user_override: SourceSelection::Skip,
            env: EnvSelection::Skip,
            ..Self::default()
        }
    }
}

/// Effective configuration together with the sources that produced it.
#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    /// Merged configuration value.
    pub value: Value,
    /// Sources considered, from lowest to highest precedence.
    pub sources: Vec<ConfigSource>,
    /// Workspace root when the project configuration declares a workspace.
    pub workspace_root: Option<PathBuf>,
    /// Root of the selected workspace member, when one was found.
    pub member_root: Option<PathBuf>,
}

fn project_config_candidates(directory: &Path) -> [PathBuf; 4] {
    [
        directory.join("morphir.toml"),
        directory.join("morphir.yaml"),
        directory.join(".morphir").join("morphir.toml"),
        directory.join(".morphir").join("morphir.yaml"),
    ]
}

/// Return the only existing candidate, or report all conflicting candidates.
pub fn discover_config_candidates(candidates: &[PathBuf]) -> Result<Option<PathBuf>> {
    let found = candidates
        .iter()
        .filter(|candidate| candidate.is_file())
        .fold(Vec::new(), |mut found, candidate| {
            if !found.contains(candidate) {
                found.push(candidate.clone());
            }
            found
        });

    match found.as_slice() {
        [] => Ok(None),
        [config] => Ok(Some(config.clone())),
        configs => {
            let paths = configs
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("Ambiguous Morphir configuration; found: {paths}")
        }
    }
}

/// Find one configuration directly inside a project or workspace directory.
pub fn discover_config_at(directory: &Path) -> Result<Option<PathBuf>> {
    let modern = discover_config_candidates(&project_config_candidates(directory))?;
    Ok(modern.or_else(|| {
        let legacy = directory.join("morphir.json");
        legacy.is_file().then_some(legacy)
    }))
}

/// Walk up the directory tree to find one project or workspace configuration.
pub fn discover_config(start_dir: &Path) -> Result<Option<PathBuf>> {
    let mut current = start_dir.to_path_buf();

    loop {
        if let Some(config) = discover_config_at(&current)? {
            return Ok(Some(config));
        }

        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }

    Ok(None)
}

/// Build all global user configuration candidates for resolved platform roots.
pub fn global_config_candidates(
    platform: ConfigPlatform,
    home_dir: Option<&Path>,
    platform_config_dir: Option<&Path>,
    xdg_config_home: Option<&Path>,
) -> Vec<PathBuf> {
    let valid_xdg = xdg_config_home.filter(|path| is_absolute_for(platform, path));
    let config_dir = match platform {
        ConfigPlatform::Xdg | ConfigPlatform::MacOs => valid_xdg.or(platform_config_dir),
        ConfigPlatform::Windows => platform_config_dir,
    };

    config_dir
        .into_iter()
        .map(|root| root.join("morphir"))
        .chain(home_dir.into_iter().map(|root| root.join(".morphir")))
        .flat_map(|root| [root.join("morphir.toml"), root.join("morphir.yaml")])
        .collect()
}

fn is_absolute_for(platform: ConfigPlatform, path: &Path) -> bool {
    let value = path.to_string_lossy();
    match platform {
        ConfigPlatform::Xdg | ConfigPlatform::MacOs => value.starts_with('/'),
        ConfigPlatform::Windows => {
            value.starts_with(r"\\")
                || (value.as_bytes().get(1) == Some(&b':')
                    && matches!(value.as_bytes().get(2), Some(b'\\' | b'/')))
        }
    }
}

fn current_platform() -> ConfigPlatform {
    #[cfg(target_os = "windows")]
    return ConfigPlatform::Windows;
    #[cfg(target_os = "macos")]
    return ConfigPlatform::MacOs;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    ConfigPlatform::Xdg
}

fn native_global_config_candidates() -> Vec<PathBuf> {
    let platform = current_platform();
    let home_dir = dirs::home_dir();
    let platform_config_dir = match platform {
        ConfigPlatform::Xdg => home_dir.as_ref().map(|home| home.join(".config")),
        ConfigPlatform::MacOs | ConfigPlatform::Windows => dirs::config_dir(),
    };
    let xdg_config_home = match platform {
        ConfigPlatform::Xdg | ConfigPlatform::MacOs => {
            std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
        }
        ConfigPlatform::Windows => None,
    };
    global_config_candidates(
        platform,
        home_dir.as_deref(),
        platform_config_dir.as_deref(),
        xdg_config_home.as_deref(),
    )
}

/// Discover the global user configuration using native platform directories.
pub fn discover_global_config() -> Result<Option<PathBuf>> {
    discover_config_candidates(&native_global_config_candidates())
}

/// Return the system configuration directory for a platform.
///
/// Unix-like systems use `/etc`; Windows uses `%PROGRAMDATA%` and falls back to
/// `C:\ProgramData` when the variable is not set.
pub fn default_system_config_dir(platform: ConfigPlatform, program_data: Option<&Path>) -> PathBuf {
    match platform {
        ConfigPlatform::Windows => program_data
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData")),
        ConfigPlatform::Xdg | ConfigPlatform::MacOs => PathBuf::from("/etc"),
    }
}

/// Build the system configuration candidates below a system configuration directory.
pub fn system_config_candidates(system_config_dir: &Path) -> [PathBuf; 2] {
    let root = system_config_dir.join("morphir");
    [root.join("morphir.toml"), root.join("morphir.yaml")]
}

fn native_system_config_candidates() -> [PathBuf; 2] {
    let platform = current_platform();
    let program_data = match platform {
        ConfigPlatform::Windows => std::env::var_os("PROGRAMDATA").map(PathBuf::from),
        ConfigPlatform::Xdg | ConfigPlatform::MacOs => None,
    };
    system_config_candidates(&default_system_config_dir(
        platform,
        program_data.as_deref(),
    ))
}

/// Discover the system configuration using native platform directories.
pub fn discover_system_config() -> Result<Option<PathBuf>> {
    discover_config_candidates(&native_system_config_candidates())
}

/// Build the user override candidates for a project root.
pub fn user_override_candidates(project_root: &Path) -> [PathBuf; 2] {
    let root = project_root.join(".morphir");
    [
        root.join("morphir.user.toml"),
        root.join("morphir.user.yaml"),
    ]
}

/// Find the user override for a project root, rejecting sibling TOML and YAML files.
pub fn discover_user_override(project_root: &Path) -> Result<Option<PathBuf>> {
    discover_config_candidates(&user_override_candidates(project_root))
}

/// Return the project or workspace root represented by a configuration path.
pub fn config_root(config_path: &Path) -> Option<&Path> {
    let parent = config_path.parent()?;
    let is_hidden_config = parent.file_name().and_then(|name| name.to_str()) == Some(".morphir")
        && matches!(
            config_path.file_name().and_then(|name| name.to_str()),
            Some("morphir.toml" | "morphir.yaml")
        );

    if is_hidden_config {
        parent.parent()
    } else {
        Some(parent)
    }
}

/// Walk up directory tree to find `.morphir/` directory
pub fn discover_morphir_dir(start_dir: &Path) -> Option<PathBuf> {
    let mut current = start_dir.to_path_buf();

    loop {
        let morphir_path = current.join(".morphir");
        if morphir_path.exists() && morphir_path.is_dir() {
            return Some(morphir_path);
        }

        // Move up one directory
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }

    None
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

fn merge_source(effective: &mut Value, source: &ConfigSource) -> Result<()> {
    if let (ConfigSourceStatus::Loaded, Some(path)) = (source.status, &source.path) {
        let layer = load_config_value(path).with_context(|| {
            format!(
                "Failed to load {} configuration: {}",
                source.kind.name(),
                path.display()
            )
        })?;
        *effective = deep_merge(effective, &layer);
    }
    Ok(())
}

fn decode_config(value: &Value, what: &str) -> Result<MorphirConfig> {
    serde_json::from_value(value.clone())
        .with_context(|| format!("Failed to decode {what} Morphir config"))
}

fn env_source(effective: &mut Value, options: &ConfigLoadOptions) -> ConfigSource {
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
    *effective = deep_merge(effective, &layer);
    ConfigSource::new(ConfigSourceKind::Environment, None, Vec::new(), status)
}

/// Compute the effective configuration from every configured source.
///
/// Sources are merged from lowest to highest precedence: built-in defaults,
/// system, global user, project, selected workspace member, user override(s),
/// and environment variables. `project_config` is the discovered or explicitly
/// selected project configuration; pass `None` to inspect the non-project
/// layers alone.
pub fn load_effective_config(
    project_config: Option<&Path>,
    options: &ConfigLoadOptions,
) -> Result<EffectiveConfig> {
    let mut effective = Value::Object(Map::new());
    let mut sources = vec![ConfigSource::new(
        ConfigSourceKind::Defaults,
        None,
        Vec::new(),
        ConfigSourceStatus::Loaded,
    )];

    let system = resolve_file_source(ConfigSourceKind::System, &options.system, || {
        native_system_config_candidates().to_vec()
    })?;
    merge_source(&mut effective, &system)?;
    sources.push(system);

    let global = resolve_file_source(
        ConfigSourceKind::Global,
        &options.global,
        native_global_config_candidates,
    )?;
    merge_source(&mut effective, &global)?;
    sources.push(global);

    let project = match project_config {
        Some(path) => ConfigSource::loaded(ConfigSourceKind::Project, path.to_path_buf()),
        None => ConfigSource::not_found(ConfigSourceKind::Project, Vec::new()),
    };
    merge_source(&mut effective, &project)?;
    sources.push(project);

    let project_root = project_config.and_then(config_root).map(Path::to_path_buf);
    let root_config = decode_config(&effective, "project")?;
    let workspace_root = root_config
        .is_workspace()
        .then(|| project_root.clone())
        .flatten();

    let member_root = match (&workspace_root, &root_config.workspace) {
        (Some(ws_root), Some(ws)) => {
            // Resolve the default member path (glob patterns are treated literally for now)
            let member = ws.default_member.as_ref().or_else(|| ws.members.first());
            match member {
                Some(member) => {
                    let member_path = ws_root.join(member);
                    match discover_config_at(&member_path)? {
                        Some(member_config) => {
                            let source = ConfigSource::loaded(
                                ConfigSourceKind::WorkspaceMember,
                                member_config,
                            );
                            merge_source(&mut effective, &source)?;
                            sources.push(source);
                            Some(member_path)
                        }
                        None => {
                            sources.push(ConfigSource::not_found(
                                ConfigSourceKind::WorkspaceMember,
                                project_config_candidates(&member_path).to_vec(),
                            ));
                            None
                        }
                    }
                }
                None => None,
            }
        }
        _ => None,
    };

    match &options.user_override {
        SourceSelection::Discover => {
            let mut roots = project_root.iter().collect::<Vec<_>>();
            if let Some(member_root) = &member_root
                && Some(member_root) != project_root.as_ref()
            {
                roots.push(member_root);
            }
            if roots.is_empty() {
                sources.push(ConfigSource::skipped(ConfigSourceKind::UserOverride));
            }
            for root in roots {
                let source = resolve_file_source(
                    ConfigSourceKind::UserOverride,
                    &SourceSelection::Discover,
                    || user_override_candidates(root).to_vec(),
                )?;
                merge_source(&mut effective, &source)?;
                sources.push(source);
            }
        }
        selection => {
            let source = resolve_file_source(ConfigSourceKind::UserOverride, selection, Vec::new)?;
            merge_source(&mut effective, &source)?;
            sources.push(source);
        }
    }

    sources.push(env_source(&mut effective, options));

    Ok(EffectiveConfig {
        value: effective,
        sources,
        workspace_root,
        member_root,
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

/// Resolve compile output path using Mill-inspired structure
pub fn resolve_compile_output(project: &str, language: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir
        .join("out")
        .join(sanitize_project_name(project))
        .join("compile")
        .join(language)
}

/// Resolve generate output path using Mill-inspired structure
pub fn resolve_generate_output(project: &str, target: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir
        .join("out")
        .join(sanitize_project_name(project))
        .join("generate")
        .join(target)
}

/// Resolve distribution output path
pub fn resolve_dist_output(project: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir
        .join("out")
        .join(sanitize_project_name(project))
        .join("dist")
}

/// Resolve test fixture path
pub fn resolve_test_fixture(name: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir.join("test").join("fixtures").join(name)
}

/// Resolve test scenario path
pub fn resolve_test_scenario(name: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir.join("test").join("scenarios").join(name)
}

/// Sanitize project name for filesystem use
pub fn sanitize_project_name(name: &str) -> String {
    // Replace invalid characters, but preserve structure
    // For now, just replace slashes and spaces
    name.replace(['/', ' ', '\\'], "-")
}

/// Resolve path relative to config file location
pub fn resolve_path_relative_to_config(path: &Path, config_path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        config_root(config_path)
            .unwrap_or(Path::new("."))
            .join(path)
    }
}

/// Resolve path relative to workspace root
pub fn resolve_path_relative_to_workspace(path: &Path, workspace_root: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

/// Ensure .morphir/ folder structure is created
pub fn ensure_morphir_structure(morphir_dir: &Path) -> Result<()> {
    // Create base directories
    std::fs::create_dir_all(morphir_dir.join("out"))?;
    std::fs::create_dir_all(morphir_dir.join("test").join("fixtures"))?;
    std::fs::create_dir_all(morphir_dir.join("test").join("scenarios"))?;
    std::fs::create_dir_all(morphir_dir.join("logs"))?;
    std::fs::create_dir_all(morphir_dir.join("cache"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_project_config(path: &Path) {
        write_file(path, "project:\n  name: Acme.Project\n  version: 1.0.0\n");
    }

    fn write_file(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().expect("config parent")).unwrap();
        std::fs::write(path, content).unwrap();
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

    #[test]
    fn discovers_yaml_while_walking_parent_directories() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("src").join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let expected = root.path().join("morphir.yaml");
        write_project_config(&expected);

        assert_eq!(discover_config(&nested).unwrap(), Some(expected));
    }

    #[test]
    fn discovers_hidden_project_config() {
        let root = tempfile::tempdir().unwrap();
        let expected = root.path().join(".morphir").join("morphir.yaml");
        write_project_config(&expected);

        assert_eq!(discover_config(root.path()).unwrap(), Some(expected));
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
    fn rejects_ambiguous_project_configs() {
        let root = tempfile::tempdir().unwrap();
        let toml = root.path().join("morphir.toml");
        let yaml = root.path().join("morphir.yaml");
        std::fs::write(&toml, "[project]\nname = \"Acme.Project\"\nversion = \"1\"").unwrap();
        write_project_config(&yaml);

        let error = discover_config(root.path()).expect_err("ambiguous config");
        let message = error.to_string();
        assert!(message.contains(toml.to_str().unwrap()));
        assert!(message.contains(yaml.to_str().unwrap()));
    }

    #[test]
    fn rejects_hidden_and_visible_project_configs_together() {
        let root = tempfile::tempdir().unwrap();
        let visible = root.path().join("morphir.yaml");
        let hidden = root.path().join(".morphir").join("morphir.yaml");
        write_project_config(&visible);
        write_project_config(&hidden);

        let error = discover_config(root.path()).expect_err("ambiguous config");
        let message = error.to_string();
        assert!(message.contains(visible.to_str().unwrap()));
        assert!(message.contains(hidden.to_str().unwrap()));
    }

    #[test]
    fn does_not_implicitly_discover_yml() {
        let root = tempfile::tempdir().unwrap();
        write_project_config(&root.path().join("morphir.yml"));

        assert_eq!(discover_config(root.path()).unwrap(), None);
    }

    #[test]
    fn resolves_linux_xdg_and_home_candidates() {
        let candidates = global_config_candidates(
            ConfigPlatform::Xdg,
            Some(Path::new("/home/alice")),
            Some(Path::new("/ignored/platform")),
            Some(Path::new("/srv/alice/config")),
        );

        assert_eq!(
            candidates,
            vec![
                PathBuf::from("/srv/alice/config/morphir/morphir.toml"),
                PathBuf::from("/srv/alice/config/morphir/morphir.yaml"),
                PathBuf::from("/home/alice/.morphir/morphir.toml"),
                PathBuf::from("/home/alice/.morphir/morphir.yaml"),
            ]
        );
    }

    #[test]
    fn ignores_relative_xdg_config_home() {
        let candidates = global_config_candidates(
            ConfigPlatform::Xdg,
            Some(Path::new("/home/alice")),
            Some(Path::new("/home/alice/.config")),
            Some(Path::new("relative/config")),
        );

        assert_eq!(
            candidates[0],
            PathBuf::from("/home/alice/.config/morphir/morphir.toml")
        );
    }

    #[test]
    fn uses_macos_application_support_and_home_candidates() {
        let candidates = global_config_candidates(
            ConfigPlatform::MacOs,
            Some(Path::new("/Users/Alice")),
            Some(Path::new("/Users/Alice/Library/Application Support")),
            None,
        );

        assert_eq!(
            candidates[0],
            PathBuf::from("/Users/Alice/Library/Application Support/morphir/morphir.toml")
        );
        assert_eq!(
            candidates[2],
            PathBuf::from("/Users/Alice/.morphir/morphir.toml")
        );
    }

    #[test]
    fn uses_windows_known_folder_candidates() {
        let candidates = global_config_candidates(
            ConfigPlatform::Windows,
            Some(Path::new(r"D:\Profiles\Alice")),
            Some(Path::new(r"D:\Profiles\Alice\Roaming")),
            Some(Path::new(r"D:\ignored-xdg")),
        );

        assert_eq!(
            candidates[0],
            PathBuf::from(r"D:\Profiles\Alice\Roaming").join("morphir/morphir.toml")
        );
        assert_eq!(
            candidates[2],
            PathBuf::from(r"D:\Profiles\Alice").join(".morphir/morphir.toml")
        );
    }

    #[test]
    fn rejects_ambiguous_global_configs() {
        let root = tempfile::tempdir().unwrap();
        let config_dir = root.path().join("config");
        let home_dir = root.path().join("home");
        let candidates = global_config_candidates(
            ConfigPlatform::Xdg,
            Some(&home_dir),
            Some(&config_dir),
            None,
        );
        std::fs::create_dir_all(candidates[0].parent().unwrap()).unwrap();
        std::fs::create_dir_all(candidates[3].parent().unwrap()).unwrap();
        std::fs::write(&candidates[0], "[morphir]\nversion = \"1\"").unwrap();
        std::fs::write(&candidates[3], "morphir:\n  version: '1'\n").unwrap();

        let error = discover_config_candidates(&candidates).expect_err("ambiguous config");
        let message = error.to_string();
        assert!(message.contains(candidates[0].to_str().unwrap()));
        assert!(message.contains(candidates[3].to_str().unwrap()));
    }

    #[test]
    fn resolves_system_config_candidates_per_platform() {
        assert_eq!(
            system_config_candidates(&default_system_config_dir(ConfigPlatform::Xdg, None)),
            [
                PathBuf::from("/etc/morphir/morphir.toml"),
                PathBuf::from("/etc/morphir/morphir.yaml"),
            ]
        );
        assert_eq!(
            default_system_config_dir(ConfigPlatform::MacOs, Some(Path::new("/ignored"))),
            PathBuf::from("/etc")
        );
        assert_eq!(
            system_config_candidates(&default_system_config_dir(
                ConfigPlatform::Windows,
                Some(Path::new(r"D:\ProgramData"))
            ))[1],
            PathBuf::from(r"D:\ProgramData").join("morphir/morphir.yaml")
        );
        assert_eq!(
            default_system_config_dir(ConfigPlatform::Windows, None),
            PathBuf::from(r"C:\ProgramData")
        );
    }

    #[test]
    fn discovers_user_override_and_rejects_sibling_serializations() {
        let root = tempfile::tempdir().unwrap();
        let toml = root.path().join(".morphir").join("morphir.user.toml");
        let yaml = root.path().join(".morphir").join("morphir.user.yaml");

        assert_eq!(discover_user_override(root.path()).unwrap(), None);

        write_file(&yaml, "ui:\n  theme: dark\n");
        assert_eq!(
            discover_user_override(root.path()).unwrap(),
            Some(yaml.clone())
        );

        write_file(&toml, "[ui]\ntheme = \"light\"\n");
        let error = discover_user_override(root.path()).expect_err("ambiguous override");
        let message = error.to_string();
        assert!(message.contains(toml.to_str().unwrap()));
        assert!(message.contains(yaml.to_str().unwrap()));
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
        let user = root
            .path()
            .join("project")
            .join(".morphir")
            .join("morphir.user.toml");
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
            "[logging]\nfile = \"debug.log\"\n\n[codegen]\ntargets = [\"typescript\"]\n",
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
            json!({
                "project": {"name": "Acme.Project", "version": "1.0.0"},
                "logging": {"level": "error", "format": "json", "file": "debug.log"},
                "ui": {"color": false, "theme": "dark"},
                "cache": {"enabled": false},
                "codegen": {"targets": ["typescript"]},
                "ir": {"strict_mode": true},
            })
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
        assert_eq!(user.candidates, user_override_candidates(root.path()));
        assert_eq!(
            source(&context.sources, ConfigSourceKind::Environment).status,
            ConfigSourceStatus::NotFound
        );
        assert_eq!(context.config.project.unwrap().name, "Acme.Project");
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
            json!({"ui": {"theme": "dark", "color": false}})
        );
        assert_eq!(
            source(&effective.sources, ConfigSourceKind::Project).status,
            ConfigSourceStatus::NotFound
        );
        assert_eq!(
            source(&effective.sources, ConfigSourceKind::UserOverride).status,
            ConfigSourceStatus::Skipped
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
        let workspace_user = root.path().join(".morphir").join("morphir.user.yaml");
        let member_user = root
            .path()
            .join("packages")
            .join("orders")
            .join(".morphir")
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
            env: env(&[]),
            ..ConfigLoadOptions::project_only()
        };
        let options = ConfigLoadOptions {
            user_override: SourceSelection::Discover,
            ..options
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
    fn source_locations_describe_each_kind() {
        let defaults = ConfigSource::new(
            ConfigSourceKind::Defaults,
            None,
            Vec::new(),
            ConfigSourceStatus::Loaded,
        );
        assert_eq!(defaults.location(), "(built-in)");
        assert_eq!(
            ConfigSource::skipped(ConfigSourceKind::Environment).location(),
            "MORPHIR_*"
        );
        let missing = ConfigSource::not_found(
            ConfigSourceKind::Global,
            vec![
                PathBuf::from("/a/morphir.toml"),
                PathBuf::from("/a/morphir.yaml"),
            ],
        );
        assert_eq!(missing.location(), "/a/morphir.toml, /a/morphir.yaml");
        assert_eq!(missing.priority, 200);
    }
}

//! Configuration sources: the layers that make up the effective configuration,
//! how callers select them, and what the loader reports about each one.

use super::provenance::{ConfigOrigin, ConfigProvenance};
use morphir_common::config::env::DEFAULT_ENV_PREFIX;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

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
    /// Personal override adjacent to a selected standard primary configuration.
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
    pub(crate) fn new(
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

    pub(crate) fn loaded(kind: ConfigSourceKind, path: PathBuf) -> Self {
        Self::new(
            kind,
            Some(path.clone()),
            vec![path],
            ConfigSourceStatus::Loaded,
        )
    }

    pub(crate) fn not_found(kind: ConfigSourceKind, candidates: Vec<PathBuf>) -> Self {
        Self::new(kind, None, candidates, ConfigSourceStatus::NotFound)
    }

    pub(crate) fn skipped(kind: ConfigSourceKind) -> Self {
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
    /// User override selection. `Discover` looks beside the selected project
    /// primary and, when selected, beside the workspace-member primary.
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
    /// Files belonging to the selected member that set `workspace.out_dir`,
    /// which was dropped from their layer because the out root belongs to the
    /// whole workspace.
    pub(crate) ignored_member_out_dir: Vec<PathBuf>,
    /// Warnings raised while resolving the workspace and its members, such as
    /// a `members` entry that would leave the workspace directory.
    pub(crate) warnings: Vec<String>,
    /// Origins of the winning configuration values.
    pub(crate) provenance: ConfigProvenance,
}

impl EffectiveConfig {
    pub(crate) fn origin_for_key(&self, key: &str) -> Option<&ConfigOrigin> {
        self.provenance.origin(&key_path(key))
    }
}

pub(crate) fn key_path(key: &str) -> Vec<String> {
    key.split('.').map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            ConfigSource::loaded(ConfigSourceKind::Project, PathBuf::from("/p/morphir.toml"))
                .location(),
            "/p/morphir.toml"
        );
    }

    #[test]
    fn precedence_increases_with_kind_order() {
        let kinds = [
            ConfigSourceKind::Defaults,
            ConfigSourceKind::System,
            ConfigSourceKind::Global,
            ConfigSourceKind::Project,
            ConfigSourceKind::WorkspaceMember,
            ConfigSourceKind::UserOverride,
            ConfigSourceKind::Environment,
        ];
        assert!(
            kinds
                .windows(2)
                .all(|pair| pair[0].priority() < pair[1].priority())
        );
        assert!(kinds.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn project_only_options_skip_every_other_layer() {
        let options = ConfigLoadOptions::project_only();
        assert_eq!(options.system, SourceSelection::Skip);
        assert_eq!(options.global, SourceSelection::Skip);
        assert_eq!(options.user_override, SourceSelection::Skip);
        assert_eq!(options.env, EnvSelection::Skip);
        assert_eq!(options.env_prefix, DEFAULT_ENV_PREFIX);
    }
}

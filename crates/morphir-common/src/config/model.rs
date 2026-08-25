use crate::remote::config::RemoteSourceConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Root configuration from morphir.toml
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MorphirConfig {
    /// Morphir toolchain settings
    #[serde(default)]
    pub morphir: Option<MorphirSection>,

    /// Project configuration (for project mode)
    #[serde(default)]
    pub project: Option<ProjectSection>,

    /// Workspace configuration (for workspace mode)
    #[serde(default)]
    pub workspace: Option<WorkspaceSection>,

    /// Frontend/language configuration
    #[serde(default)]
    pub frontend: Option<FrontendSection>,

    /// IR format settings
    #[serde(default)]
    pub ir: Option<IrSection>,

    /// Code generation settings
    #[serde(default)]
    pub codegen: Option<CodegenSection>,

    /// Remote source configuration
    #[serde(default)]
    pub sources: Option<RemoteSourceConfig>,

    /// Dependencies
    #[serde(default)]
    pub dependencies: HashMap<String, DependencySpec>,

    /// Dev dependencies
    #[serde(default, rename = "dev-dependencies")]
    pub dev_dependencies: HashMap<String, DependencySpec>,

    /// Extensions
    #[serde(default)]
    pub extensions: HashMap<String, ExtensionSpec>,

    /// Tasks
    #[serde(default)]
    pub tasks: HashMap<String, TaskSpec>,
}

impl MorphirConfig {
    /// Check if this is a workspace configuration
    pub fn is_workspace(&self) -> bool {
        self.workspace.is_some()
    }

    /// Check if this is a project configuration
    pub fn is_project(&self) -> bool {
        self.project.is_some()
    }
}

/// [morphir] section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MorphirSection {
    /// Required IR version constraint
    pub version: String,
    /// Minimum CLI version
    pub min_cli_version: Option<String>,
    /// Enable dev mode (run from source instead of installed binary)
    #[serde(default)]
    pub dev_mode: bool,
}

/// [project] section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSection {
    /// Package name (org/name format)
    pub name: String,
    /// Semantic version
    pub version: String,
    /// Description
    pub description: Option<String>,
    /// Authors
    #[serde(default)]
    pub authors: Vec<String>,
    /// License (SPDX)
    pub license: Option<String>,
    /// Repository URL
    pub repository: Option<String>,
    /// Source directory
    #[serde(default = "default_source_dir")]
    pub source_directory: String,
    /// Exposed modules
    #[serde(default)]
    pub exposed_modules: Vec<String>,
    /// Output directory
    #[serde(default = "default_output_dir")]
    pub output_directory: String,
}

pub(crate) fn default_source_dir() -> String {
    "src".to_string()
}

pub(crate) fn default_output_dir() -> String {
    ".morphir/out".to_string()
}

/// [workspace] section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSection {
    /// Glob patterns for member discovery
    #[serde(default)]
    pub members: Vec<String>,
    /// Patterns to exclude
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Default member for commands
    pub default_member: Option<String>,
    /// Workspace output directory
    #[serde(default = "default_workspace_output")]
    pub output_dir: String,
}

fn default_workspace_output() -> String {
    ".morphir".to_string()
}

/// [frontend] section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendSection {
    /// Default source language
    pub language: Option<String>,
    /// Emit parse stage output as JSON (default: true)
    /// When enabled, writes parsed AST to .morphir/out/<project>/parse/<module>.json
    #[serde(default = "default_true")]
    pub emit_parse_stage: bool,
    /// Treat parse stage emission failures as fatal errors (default: false)
    /// When true, compilation fails if parse stage output cannot be written
    /// When false, failures are logged as warnings but compilation continues
    #[serde(default)]
    pub emit_parse_stage_fatal: bool,
    /// Language-specific settings
    #[serde(flatten)]
    pub settings: HashMap<String, toml::Value>,
}

/// [ir] section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrSection {
    /// IR format version
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    /// Output mode (classic or vfs)
    #[serde(default = "default_ir_mode")]
    pub mode: String,
    /// Strict mode
    #[serde(default)]
    pub strict_mode: bool,
}

/// IR format version used when a configuration does not set one.
///
/// Version 4 is the default and where active development happens. Version 3
/// remains supported: a project pins it with `ir.format_version = 3`, and the
/// tests below cover that path so it does not rot while v4 moves.
fn default_format_version() -> u32 {
    4
}

fn default_ir_mode() -> String {
    "vfs".to_string()
}

/// [codegen] section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodegenSection {
    /// Code generation targets
    #[serde(default)]
    pub targets: Vec<String>,
    /// Output format
    #[serde(default = "default_output_format")]
    pub output_format: String,
    /// Target-specific settings
    #[serde(flatten)]
    pub settings: HashMap<String, toml::Value>,
}

fn default_output_format() -> String {
    "pretty".to_string()
}

/// Dependency specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DependencySpec {
    /// Simple version string
    Version(String),
    /// Detailed specification
    Detailed(DetailedDependency),
}

/// Detailed dependency specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailedDependency {
    /// Version constraint
    pub version: Option<String>,
    /// Local path
    pub path: Option<PathBuf>,
    /// Git URL
    pub git: Option<String>,
    /// Git tag
    pub tag: Option<String>,
    /// Git branch
    pub branch: Option<String>,
    /// Git revision
    pub rev: Option<String>,
    /// Workspace inheritance
    pub workspace: Option<bool>,
}

/// Extension specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionSpec {
    /// Local path to WASM file
    pub path: Option<PathBuf>,
    /// URL to download
    pub url: Option<String>,
    /// Command for native extension
    pub command: Option<String>,
    /// Command arguments
    #[serde(default)]
    pub args: Vec<String>,
    /// Enable/disable
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Extension-specific config
    #[serde(default)]
    pub config: HashMap<String, toml::Value>,
}

fn default_true() -> bool {
    true
}

/// Task specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TaskSpec {
    /// Simple command string
    Simple(String),
    /// Detailed task
    Detailed(DetailedTask),
}

/// Detailed task specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailedTask {
    /// Description
    pub description: Option<String>,
    /// Command to run
    pub run: Option<String>,
    /// Task dependencies
    #[serde(default)]
    pub depends: Vec<String>,
    /// Working directory
    pub cwd: Option<PathBuf>,
    /// Environment variables
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Version 4 is the default, so a config that says nothing about the IR
    /// gets 4. This is the pin: changing the default fails here first.
    #[test]
    fn ir_format_version_defaults_to_4() {
        let ir: IrSection = serde_json::from_value(json!({})).expect("empty ir section");
        assert_eq!(ir.format_version, 4);
    }

    /// Version 3 stays selectable while v4 is under development. A project
    /// that pins 3 must get 3 — not the default, and not a coerced 4.
    #[test]
    fn ir_format_version_3_is_honoured_when_pinned() {
        let ir: IrSection =
            serde_json::from_value(json!({"format_version": 3})).expect("v3 ir section");
        assert_eq!(ir.format_version, 3);

        let config: MorphirConfig =
            serde_json::from_value(json!({"ir": {"format_version": 3, "strict_mode": true}}))
                .expect("v3 config");
        let ir = config.ir.expect("ir section");
        assert_eq!(ir.format_version, 3);
        assert!(ir.strict_mode);
        // Pinning the version must not disturb the other IR settings.
        assert_eq!(ir.mode, default_ir_mode());
    }

    /// Every version in the supported range decodes, so v3 is not a special
    /// case that happens to work: v1 and v2 configs remain loadable too.
    #[test]
    fn every_supported_ir_format_version_decodes() {
        for version in 1..=10u32 {
            let ir: IrSection = serde_json::from_value(json!({"format_version": version}))
                .unwrap_or_else(|error| panic!("format_version {version}: {error}"));
            assert_eq!(ir.format_version, version);
        }
    }

    /// The IR section round-trips, so a pinned v3 survives being written back
    /// out — the path `morphir config show` and workspace tooling depend on.
    #[test]
    fn ir_section_round_trips_through_serde() {
        let original: IrSection =
            serde_json::from_value(json!({"format_version": 3, "mode": "classic"}))
                .expect("v3 ir section");
        let reparsed: IrSection =
            serde_json::from_value(serde_json::to_value(&original).expect("serialize"))
                .expect("deserialize");
        assert_eq!(reparsed.format_version, 3);
        assert_eq!(reparsed.mode, "classic");
    }
}

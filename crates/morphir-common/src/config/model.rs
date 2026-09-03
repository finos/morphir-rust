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
}

pub(crate) fn default_source_dir() -> String {
    "src".to_string()
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
    /// Out directory for every task in the workspace, relative to the
    /// workspace root.
    #[serde(default = "default_workspace_out_dir")]
    pub out_dir: String,
}

fn default_workspace_out_dir() -> String {
    ".morphir/out".to_string()
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
    /// Storage layout compile writes: `single-file` or `document-tree`
    #[serde(
        default = "default_ir_layout",
        deserialize_with = "deserialize_ir_layout"
    )]
    pub layout: String,
    /// Serialization format compile writes: `json` or `yaml`
    #[serde(
        default = "default_ir_format",
        deserialize_with = "deserialize_ir_format"
    )]
    pub format: String,
    /// Deprecated alias for `layout`: `classic` means `single-file`, `vfs`
    /// means `document-tree`. Read for one release, then removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
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

fn default_ir_layout() -> String {
    "single-file".to_string()
}

fn default_ir_format() -> String {
    "json".to_string()
}

/// The values `ir.layout` accepts. The JSON schema (`morphir-config-v1.json`)
/// restricts the field the same way; keep the two in agreement.
const IR_LAYOUT_VALUES: [&str; 2] = ["single-file", "document-tree"];

/// The values `ir.format` accepts. The JSON schema (`morphir-config-v1.json`)
/// restricts the field the same way; keep the two in agreement.
const IR_FORMAT_VALUES: [&str; 2] = ["json", "yaml"];

fn deserialize_ir_layout<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_one_of("ir.layout", &IR_LAYOUT_VALUES, deserializer)
}

fn deserialize_ir_format<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_one_of("ir.format", &IR_FORMAT_VALUES, deserializer)
}

/// Deserialize a string field, rejecting anything outside `accepted`.
fn deserialize_one_of<'de, D>(
    field: &str,
    accepted: &[&str],
    deserializer: D,
) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if accepted.contains(&value.as_str()) {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(format!(
            "{field} is set to {value:?}, which is not one of the accepted values: {}",
            accepted.join(", ")
        )))
    }
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
        assert_eq!(ir.layout, default_ir_layout());
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
            serde_json::from_value(json!({"format_version": 3, "layout": "document-tree"}))
                .expect("v3 ir section");
        let reparsed: IrSection =
            serde_json::from_value(serde_json::to_value(&original).expect("serialize"))
                .expect("deserialize");
        assert_eq!(reparsed.format_version, 3);
        assert_eq!(reparsed.layout, "document-tree");
    }

    #[test]
    fn workspace_out_dir_defaults_to_dot_morphir_out() {
        let config: MorphirConfig =
            serde_json::from_value(serde_json::json!({"workspace": {"members": []}})).unwrap();
        assert_eq!(config.workspace.unwrap().out_dir, ".morphir/out");
    }

    #[test]
    fn ir_layout_and_format_default_to_single_file_json() {
        let config: MorphirConfig = serde_json::from_value(serde_json::json!({"ir": {}})).unwrap();
        let ir = config.ir.unwrap();
        assert_eq!(ir.layout, "single-file");
        assert_eq!(ir.format, "json");
        assert_eq!(ir.mode, None);
    }

    #[test]
    fn project_section_has_no_output_directory() {
        let config: MorphirConfig = serde_json::from_value(serde_json::json!({
            "project": {"name": "acme/app", "version": "1.0.0"}
        }))
        .unwrap();
        let value = serde_json::to_value(config.project.unwrap()).unwrap();
        assert!(value.get("output_directory").is_none());
    }

    /// A typo in `ir.layout` must fail to decode, naming the field, the
    /// value, and the two accepted spellings, rather than passing through
    /// and failing only when a task tries to act on it.
    #[test]
    fn an_unknown_ir_layout_value_fails_to_decode() {
        let error = serde_json::from_value::<IrSection>(json!({"layout": "single-fiel"}))
            .expect_err("typo'd layout must be rejected");
        let message = error.to_string();
        assert!(message.contains("ir.layout"), "{message}");
        assert!(message.contains("single-fiel"), "{message}");
        assert!(message.contains("single-file"), "{message}");
        assert!(message.contains("document-tree"), "{message}");
    }

    /// Same guarantee for `ir.format`.
    #[test]
    fn an_unknown_ir_format_value_fails_to_decode() {
        let error = serde_json::from_value::<IrSection>(json!({"format": "yml"}))
            .expect_err("typo'd format must be rejected");
        let message = error.to_string();
        assert!(message.contains("ir.format"), "{message}");
        assert!(message.contains("yml"), "{message}");
        assert!(message.contains("json"), "{message}");
        assert!(message.contains("yaml"), "{message}");
    }
}

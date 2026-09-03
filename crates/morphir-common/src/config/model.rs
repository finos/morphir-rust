use crate::remote::config::RemoteSourceConfig;
use morphir_workspace::RelativePath;
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
    /// workspace root, must stay inside it: no absolute path, no `..`, and no
    /// backslash separators.
    #[serde(
        default = "default_workspace_out_dir",
        deserialize_with = "deserialize_workspace_out_dir"
    )]
    pub out_dir: String,
}

fn default_workspace_out_dir() -> String {
    ".morphir/out".to_string()
}

/// Deserialize `[workspace].out_dir`, rejecting anything that is not a
/// relative path confined to the workspace.
///
/// This is a load-time property of the *configured* value only. The
/// `--out-dir` flag and `MORPHIR_OUT_DIR` are explicit, resolved-at-runtime
/// choices and keep their absolute-path behavior; they never go through this
/// deserializer.
fn deserialize_workspace_out_dir<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    RelativePath::parse(value.clone()).map_err(|_| {
        serde::de::Error::custom(format!(
            "workspace.out_dir is set to {value:?}, which must be a path relative to the \
             workspace root and confined to it: no absolute path, no `..`, and no backslash \
             separators"
        ))
    })?;
    Ok(value)
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
///
/// Decoding applies the deprecated `ir.mode` alias: a section that sets
/// `mode` and not `layout` comes back with `layout` already mapped, and
/// `mode` emptied. Every loader therefore agrees on the layout — the devkit
/// loader, which normalises the alias per configuration layer before merging,
/// and a plain `MorphirConfig::load` or a bare `serde_json::from_value`
/// alike. A section that sets both keeps its explicit `layout`; the devkit
/// loader is the one that warns about that, since it is the one that knows
/// which file said it.
#[derive(Debug, Clone, Serialize)]
pub struct IrSection {
    /// IR format version
    pub format_version: u32,
    /// Storage layout compile writes: `single-file` or `document-tree`
    pub layout: String,
    /// Serialization format compile writes: `json` or `yaml`
    pub format: String,
    /// Deprecated alias for `layout`: `classic` means `single-file`, `vfs`
    /// means `document-tree`. Read for one release, then removed.
    ///
    /// Always `None` on a decoded section, because decoding folds it into
    /// `layout`. It is never serialized either, so a section that is read and
    /// written back out carries `layout` alone and never both spellings of
    /// the same setting.
    #[serde(skip_serializing)]
    pub mode: Option<String>,
    /// Strict mode
    pub strict_mode: bool,
}

impl<'de> Deserialize<'de> for IrSection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        /// The section exactly as written, with `layout` optional so that
        /// "not set" can be told apart from "set to the default value" — the
        /// distinction the alias turns on.
        #[derive(Deserialize)]
        struct Written {
            #[serde(default = "default_format_version")]
            format_version: u32,
            #[serde(default, deserialize_with = "deserialize_optional_ir_layout")]
            layout: Option<String>,
            #[serde(
                default = "default_ir_format",
                deserialize_with = "deserialize_ir_format"
            )]
            format: String,
            #[serde(default)]
            mode: Option<String>,
            #[serde(default)]
            strict_mode: bool,
        }

        let written = Written::deserialize(deserializer)?;
        let layout = match (written.layout, written.mode.as_deref()) {
            (Some(layout), _) => layout,
            (None, Some(mode)) => ir_layout_for_mode(mode)
                .map_err(serde::de::Error::custom)?
                .to_owned(),
            (None, None) => default_ir_layout(),
        };
        Ok(Self {
            format_version: written.format_version,
            layout,
            format: written.format,
            mode: None,
            strict_mode: written.strict_mode,
        })
    }
}

/// The layout the deprecated `ir.mode` alias stands for, or a message saying
/// the value is not one of the two spellings it accepts.
pub fn ir_layout_for_mode(mode: &str) -> Result<&'static str, String> {
    match mode {
        "classic" => Ok("single-file"),
        "vfs" => Ok("document-tree"),
        other => Err(format!(
            "ir.mode is set to {other:?}, which is not a recognized value; \
             use \"classic\" or \"vfs\""
        )),
    }
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

/// `ir.layout` when the caller needs to know whether it was written at all.
/// An absent key stays `None`; a present one is checked the same way
/// [`deserialize_ir_layout`] checks it.
fn deserialize_optional_ir_layout<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_ir_layout(deserializer).map(Some)
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
    fn workspace_out_dir_accepts_a_normal_relative_value() {
        let config: MorphirConfig = serde_json::from_value(
            serde_json::json!({"workspace": {"members": [], "out_dir": "build/out"}}),
        )
        .unwrap();
        assert_eq!(config.workspace.unwrap().out_dir, "build/out");
    }

    /// `workspace.out_dir` must stay inside the workspace: an absolute path
    /// would place task output, and future cleanup, outside it.
    #[test]
    fn workspace_out_dir_rejects_an_absolute_path() {
        let error = serde_json::from_value::<MorphirConfig>(
            serde_json::json!({"workspace": {"members": [], "out_dir": "/tmp/out"}}),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("workspace.out_dir"),
            "unexpected error: {error}"
        );
    }

    /// `..` in `workspace.out_dir` would let the value escape the workspace
    /// root, the same confinement rule member paths already enforce.
    #[test]
    fn workspace_out_dir_rejects_dot_dot() {
        let error = serde_json::from_value::<MorphirConfig>(
            serde_json::json!({"workspace": {"members": [], "out_dir": "../out"}}),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("workspace.out_dir"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn ir_layout_and_format_default_to_single_file_json() {
        let config: MorphirConfig = serde_json::from_value(serde_json::json!({"ir": {}})).unwrap();
        let ir = config.ir.unwrap();
        assert_eq!(ir.layout, "single-file");
        assert_eq!(ir.format, "json");
        assert_eq!(ir.mode, None);
    }

    /// Every loader has to agree on the layout, not just the devkit one. A
    /// bare `MorphirConfig::load` used to hand back `layout = "single-file"`
    /// with `mode = Some("vfs")` sitting beside it, so a caller that read
    /// `layout` acted on the wrong storage layout entirely.
    #[test]
    fn mode_is_applied_by_a_plain_config_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("morphir.toml");
        std::fs::write(
            &path,
            "[project]\nname = \"acme/app\"\nversion = \"1.0.0\"\n\n[ir]\nmode = \"vfs\"\n",
        )
        .unwrap();

        let ir = MorphirConfig::load(&path).unwrap().ir.expect("ir section");
        assert_eq!(ir.layout, "document-tree");
        assert_eq!(ir.mode, None, "the alias is folded in, not carried along");

        // Writing the section back out names the layout once and never the
        // alias as well.
        let written = serde_json::to_value(&ir).unwrap();
        assert_eq!(written["layout"], "document-tree");
        assert!(written.get("mode").is_none(), "{written}");
    }

    /// An explicit `layout` in the same section wins over the alias beside it.
    #[test]
    fn an_explicit_layout_beats_a_mode_in_the_same_section() {
        let ir: IrSection =
            serde_json::from_value(json!({"mode": "vfs", "layout": "single-file"})).unwrap();
        assert_eq!(ir.layout, "single-file");
    }

    /// A misspelled alias fails to decode, naming the value and the two
    /// spellings it accepts, rather than falling through to the default.
    #[test]
    fn an_unknown_mode_value_fails_to_decode() {
        let error = serde_json::from_value::<IrSection>(json!({"mode": "vfss"}))
            .expect_err("typo'd mode must be rejected");
        let message = error.to_string();
        assert!(message.contains("ir.mode"), "{message}");
        assert!(message.contains("vfss"), "{message}");
        assert!(message.contains("classic"), "{message}");
        assert!(message.contains("vfs"), "{message}");
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

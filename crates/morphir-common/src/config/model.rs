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
    ".morphir-dist".to_string()
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

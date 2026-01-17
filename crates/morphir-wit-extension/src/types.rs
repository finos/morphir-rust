//! Core types for the Morphir extension system

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Extension type/capability
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtensionType {
    /// Frontend - parses source into IR
    Frontend,
    /// Backend - generates code from IR
    Backend,
    /// Transform - transforms IR to IR
    Transform,
    /// Analyzer - analyzes IR and produces diagnostics
    Analyzer,
}

/// Information about an extension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionInfo {
    /// Extension identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Version
    pub version: String,
    /// Description
    pub description: Option<String>,
    /// Capabilities this extension provides
    pub types: Vec<ExtensionType>,
    /// Author
    pub author: Option<String>,
    /// Homepage URL
    pub homepage: Option<String>,
    /// License
    pub license: Option<String>,
}

/// Extension capabilities for runtime negotiation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtensionCapabilities {
    /// Supports streaming/incremental processing
    pub streaming: bool,
    /// Supports incremental compilation
    pub incremental: bool,
    /// Supports cancellation
    pub cancellation: bool,
    /// Supports progress reporting
    pub progress: bool,
    /// Additional capability flags
    #[serde(flatten)]
    pub extra: HashMap<String, bool>,
}

/// Resource limits for extension execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum memory in bytes
    pub max_memory_bytes: Option<u64>,
    /// Maximum execution time in milliseconds
    pub max_time_ms: Option<u64>,
    /// Maximum fuel (instruction count)
    pub max_fuel: Option<u64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory_bytes: Some(256 * 1024 * 1024), // 256 MB
            max_time_ms: Some(30_000),                  // 30 seconds
            max_fuel: Some(100_000_000),               // 100M instructions
        }
    }
}

/// Source of an extension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExtensionSource {
    /// Local file path
    Path(PathBuf),
    /// URL to download
    Url(String),
    /// Embedded in the binary
    Embedded(String),
}

/// A loaded extension instance
#[derive(Debug)]
pub struct LoadedExtension {
    /// Extension info
    pub info: ExtensionInfo,
    /// Source where it was loaded from
    pub source: ExtensionSource,
    /// Runtime capabilities
    pub capabilities: ExtensionCapabilities,
    /// Whether the extension is currently active
    pub active: bool,
}

/// Frontend extension interface types
pub mod frontend {
    use super::*;

    /// Options for frontend compilation
    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct CompileOptions {
        /// Source directory
        pub source_dir: Option<PathBuf>,
        /// Module path being compiled
        pub module_path: Option<String>,
        /// Whether this is incremental
        pub incremental: bool,
        /// Additional options
        #[serde(flatten)]
        pub extra: HashMap<String, serde_json::Value>,
    }

    /// Result of frontend compilation
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct CompileResult {
        /// Whether compilation succeeded
        pub success: bool,
        /// Compiled IR (JSON)
        pub ir: Option<serde_json::Value>,
        /// Diagnostics
        pub diagnostics: Vec<Diagnostic>,
    }

    /// A diagnostic message
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Diagnostic {
        /// Severity
        pub severity: DiagnosticSeverity,
        /// Error/warning code
        pub code: Option<String>,
        /// Message
        pub message: String,
        /// Source location
        pub location: Option<SourceLocation>,
    }

    /// Diagnostic severity level
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum DiagnosticSeverity {
        Error,
        Warning,
        Info,
        Hint,
    }

    /// Source code location
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SourceLocation {
        /// File path
        pub file: PathBuf,
        /// Start line (1-indexed)
        pub start_line: u32,
        /// Start column (1-indexed)
        pub start_col: u32,
        /// End line
        pub end_line: u32,
        /// End column
        pub end_col: u32,
    }
}

/// Backend extension interface types
pub mod backend {
    use super::*;

    /// Options for code generation
    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct GenerateOptions {
        /// Output directory
        pub output_dir: Option<PathBuf>,
        /// Output format (pretty, compact)
        pub format: Option<String>,
        /// Target-specific options
        #[serde(flatten)]
        pub extra: HashMap<String, serde_json::Value>,
    }

    /// Result of code generation
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct GenerateResult {
        /// Whether generation succeeded
        pub success: bool,
        /// Generated artifacts
        pub artifacts: Vec<Artifact>,
        /// Diagnostics
        pub diagnostics: Vec<super::frontend::Diagnostic>,
    }

    /// A generated artifact
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Artifact {
        /// Output path
        pub path: PathBuf,
        /// Content
        pub content: String,
    }
}

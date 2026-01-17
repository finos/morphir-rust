//! Core types for the Morphir extension system
//!
//! These types are shared between the SDK (guest) and daemon (host).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    /// Validator - analyzes IR and produces diagnostics
    Validator,
}

/// Information about an extension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionInfo {
    /// Extension identifier (e.g., "morphir-gleam-binding")
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Version (semver)
    pub version: String,
    /// Description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Capabilities this extension provides
    pub types: Vec<ExtensionType>,
    /// Author
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Homepage URL
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// License (SPDX identifier)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Minimum SDK version required
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_sdk_version: Option<String>,
}

impl Default for ExtensionInfo {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            version: "0.1.0".to_string(),
            description: None,
            types: Vec::new(),
            author: None,
            homepage: None,
            license: None,
            min_sdk_version: None,
        }
    }
}

/// Extension capabilities for runtime negotiation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtensionCapabilities {
    /// Supports streaming/incremental processing
    #[serde(default)]
    pub streaming: bool,
    /// Supports incremental compilation
    #[serde(default)]
    pub incremental: bool,
    /// Supports cancellation
    #[serde(default)]
    pub cancellation: bool,
    /// Supports progress reporting
    #[serde(default)]
    pub progress: bool,
    /// Additional capability flags
    #[serde(default, flatten)]
    pub extra: HashMap<String, bool>,
}

/// Resource limits for extension execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum memory in bytes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_bytes: Option<u64>,
    /// Maximum execution time in milliseconds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_time_ms: Option<u64>,
    /// Maximum fuel (instruction count)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fuel: Option<u64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory_bytes: Some(256 * 1024 * 1024), // 256 MB
            max_time_ms: Some(30_000),                 // 30 seconds
            max_fuel: Some(100_000_000),               // 100M instructions
        }
    }
}

/// Source file for compilation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceFile {
    /// File path (relative to workspace)
    pub path: String,
    /// File content
    pub content: String,
}

/// Request to compile source files
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompileRequest {
    /// Source files to compile
    pub sources: Vec<SourceFile>,
    /// Compilation options
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

/// Result of compilation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileResult {
    /// Whether compilation succeeded
    pub success: bool,
    /// Compiled IR (JSON)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir: Option<serde_json::Value>,
    /// Diagnostics
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

/// Request to generate code
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GenerateRequest {
    /// Input IR (JSON)
    pub ir: serde_json::Value,
    /// Generation options
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

/// Result of code generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateResult {
    /// Whether generation succeeded
    pub success: bool,
    /// Generated artifacts
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    /// Diagnostics
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

/// Request to validate IR
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidateRequest {
    /// Input IR (JSON)
    pub ir: serde_json::Value,
    /// Validation options
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

/// Result of validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateResult {
    /// Whether validation passed
    pub valid: bool,
    /// Diagnostics
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

/// Request to transform IR
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransformRequest {
    /// Input IR (JSON)
    pub ir: serde_json::Value,
    /// Transformation options
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

/// Result of transformation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformResult {
    /// Whether transformation succeeded
    pub success: bool,
    /// Transformed IR (JSON)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir: Option<serde_json::Value>,
    /// Diagnostics
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

/// A diagnostic message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Severity level
    pub severity: DiagnosticSeverity,
    /// Error/warning code
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Human-readable message
    pub message: String,
    /// Source location
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<SourceLocation>,
    /// Related information
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<RelatedInformation>,
}

/// Diagnostic severity level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    /// Error - compilation fails
    Error,
    /// Warning - may indicate problems
    Warning,
    /// Information - neutral message
    Info,
    /// Hint - suggestion for improvement
    Hint,
}

/// Source code location
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceLocation {
    /// File path
    pub file: String,
    /// Start line (1-indexed)
    #[serde(default)]
    pub start_line: u32,
    /// Start column (1-indexed)
    #[serde(default)]
    pub start_col: u32,
    /// End line
    #[serde(default)]
    pub end_line: u32,
    /// End column
    #[serde(default)]
    pub end_col: u32,
}

/// Related diagnostic information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedInformation {
    /// Location of related information
    pub location: SourceLocation,
    /// Message
    pub message: String,
}

/// A generated artifact
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    /// Output path (relative)
    pub path: String,
    /// Content (text or base64 for binary)
    pub content: String,
    /// Whether content is base64-encoded binary
    #[serde(default)]
    pub binary: bool,
}

/// Workspace information provided by host
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    /// Workspace root path
    pub root: String,
    /// Output directory path
    pub output_dir: String,
}

//! Migrate builtin extension.
//!
//! Transforms Morphir IR between different versions (v3/classic ↔ v4).

use crate::{BuiltinExtension, BuiltinInfo, ExtensionType};
use anyhow::{Context, Result, bail};
use morphir_ext_core::Envelope;
use serde::{Deserialize, Serialize};

#[cfg(target_family = "wasm")]
mod wasm;

/// Migrate extension for IR version transformation.
#[derive(Default)]
pub struct MigrateExtension;

impl BuiltinExtension for MigrateExtension {
    fn execute_native(&self, input: &Envelope) -> Result<Envelope> {
        // Parse input as MigrateRequest
        let request: MigrateRequest = input.as_json().context("Failed to parse migrate request")?;

        // Perform migration
        let result = perform_migration(request)?;

        // Return as envelope
        Envelope::json(&result).context("Failed to create response envelope")
    }

    fn info(&self) -> BuiltinInfo {
        BuiltinInfo {
            id: "migrate".to_string(),
            name: "IR Migration".to_string(),
            extension_type: ExtensionType::Transform,
            description: "Transform Morphir IR between v3/classic and v4 formats".to_string(),
        }
    }

    #[cfg(feature = "wasm")]
    fn wasm_bytes() -> Option<&'static [u8]> {
        // TODO: Embed WASM via include_bytes! in build process
        // Some(include_bytes!("../../../target/wasm32-unknown-unknown/release/morphir_builtins.wasm"))
        None
    }
}

/// Request format for migrate operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct MigrateRequest {
    /// Input IR (either Classic or V4 format)
    pub ir: serde_json::Value,
    /// Target format version ("classic", "v3", "v4", "latest")
    pub target_version: String,
    /// Whether to use expanded (non-compact) format for V4
    #[serde(default)]
    pub expanded: bool,
}

/// Response format for migrate operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct MigrateResponse {
    /// Whether migration succeeded
    pub success: bool,
    /// Migrated IR (if successful)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ir: Option<serde_json::Value>,
    /// Source format detected
    pub source_format: String,
    /// Target format produced
    pub target_format: String,
    /// Warnings during migration
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// Error message (if failed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Detect if IR is in V4 format by checking for V4-specific markers.
fn detect_ir_format(ir: &serde_json::Value) -> &'static str {
    // V4 format has formatVersion >= 4 or formatVersion starting with "4"
    if let Some(fv) = ir.get("formatVersion") {
        if let Some(n) = fv.as_i64()
            && n >= 4
        {
            return "v4";
        }
        if let Some(s) = fv.as_str()
            && s.starts_with('4')
        {
            return "v4";
        }
    }
    // Default to classic format
    "classic"
}

/// Perform the actual migration logic.
fn perform_migration(request: MigrateRequest) -> Result<MigrateResponse> {
    // Detect input format
    let source_format = detect_ir_format(&request.ir);
    let is_source_v4 = source_format == "v4";

    // Determine target format
    let target_v4 = match request.target_version.to_lowercase().as_str() {
        "latest" | "v4" | "4" => true,
        "classic" | "v3" | "3" | "v2" | "2" | "v1" | "1" => false,
        _ => {
            bail!(
                "Invalid target version '{}'. Valid: latest, v4, 4, classic, v3, 3",
                request.target_version
            );
        }
    };

    let target_format = if target_v4 { "v4" } else { "classic" };

    // Check if conversion is needed
    if is_source_v4 != target_v4 {
        // Cross-format conversion not yet implemented
        return Ok(MigrateResponse {
            success: false,
            ir: None,
            source_format: source_format.to_string(),
            target_format: target_format.to_string(),
            warnings: vec![],
            error: Some(format!(
                "{} -> {} conversion is not yet implemented. \
                 The converter module is currently being updated.",
                source_format, target_format
            )),
        });
    }

    // Same format → Just return input
    Ok(MigrateResponse {
        success: true,
        ir: Some(request.ir),
        source_format: source_format.to_string(),
        target_format: target_format.to_string(),
        warnings: vec![],
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrate_info() {
        let migrate = MigrateExtension;
        let info = migrate.info();
        assert_eq!(info.id, "migrate");
        assert_eq!(info.extension_type, ExtensionType::Transform);
    }

    #[test]
    fn test_migrate_same_format() {
        let migrate = MigrateExtension;

        // Classic format: ["Library", path, deps, package]
        let request = MigrateRequest {
            ir: serde_json::json!({
                "formatVersion": 1,
                "distribution": ["Library", [["test"]], [], {"modules": []}]
            }),
            target_version: "classic".to_string(),
            expanded: false,
        };

        let input = Envelope::json(&request).unwrap();
        let output = migrate.execute_native(&input).unwrap();
        let response: MigrateResponse = output.as_json().unwrap();

        assert!(response.success);
        assert_eq!(response.source_format, "classic");
        assert_eq!(response.target_format, "classic");
    }
}

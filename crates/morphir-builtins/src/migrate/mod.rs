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

/// Perform the actual migration logic.
fn perform_migration(request: MigrateRequest) -> Result<MigrateResponse> {
    use indexmap::IndexMap;
    use morphir_common::loader::LoadedDistribution;
    use morphir_core::ir::{classic, v4};

    let mut warnings = Vec::new();

    // Detect input format by attempting to parse
    let dist =
        serde_json::from_value::<morphir_common::loader::LoadedDistribution>(request.ir.clone());

    let loaded = match dist {
        Ok(d) => d,
        Err(e) => {
            return Ok(MigrateResponse {
                success: false,
                ir: None,
                source_format: "unknown".to_string(),
                target_format: request.target_version.clone(),
                warnings: vec![],
                error: Some(format!("Failed to parse input IR: {}", e)),
            });
        }
    };

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

    // Perform conversion based on source and target formats
    match (loaded, target_v4) {
        // Classic → V4
        (LoadedDistribution::Classic(dist), true) => {
            let classic::DistributionBody::Library(_, package_path, classic_deps, pkg) =
                dist.distribution;

            if !classic_deps.is_empty() {
                warnings.push(format!(
                    "{} dependencies in Classic format will be omitted (conversion not supported)",
                    classic_deps.len()
                ));
            }

            // TODO: Call actual converter when available
            // For now, create a placeholder V4 package
            let v4_pkg = morphir_core::ir::v4::Package {
                modules: IndexMap::new(), // TODO: Convert modules
            };

            let v4_ir = v4::IRFile {
                format_version: v4::FormatVersion::default(),
                distribution: v4::Distribution::Library(v4::LibraryContent {
                    package_name: morphir_core::ir::v4::PackageName::from(package_path),
                    dependencies: IndexMap::new(),
                    def: v4_pkg,
                }),
            };

            let ir_value = serde_json::to_value(&v4_ir)?;

            Ok(MigrateResponse {
                success: true,
                ir: Some(ir_value),
                source_format: "classic".to_string(),
                target_format: "v4".to_string(),
                warnings,
                error: None,
            })
        }

        // V4 → Classic
        (LoadedDistribution::V4(ir_file), false) => {
            let v4::Distribution::Library(lib_content) = ir_file.distribution else {
                return Ok(MigrateResponse {
                    success: false,
                    ir: None,
                    source_format: "v4".to_string(),
                    target_format: "classic".to_string(),
                    warnings: vec![],
                    error: Some(
                        "Only Library distributions can be converted to Classic".to_string(),
                    ),
                });
            };

            if !lib_content.dependencies.is_empty() {
                warnings.push(format!(
                    "{} dependencies in V4 format will be omitted (conversion not supported)",
                    lib_content.dependencies.len()
                ));
            }

            // TODO: Call actual converter when available
            let classic_pkg = morphir_core::ir::classic::Package {
                modules: IndexMap::new(), // TODO: Convert modules
            };

            let classic_dist = classic::Distribution {
                format_version: 1,
                distribution: classic::DistributionBody::Library(
                    classic::LibraryTag::Library,
                    lib_content.package_name.into_path(),
                    vec![],
                    classic_pkg,
                ),
            };

            let ir_value = serde_json::to_value(&classic_dist)?;

            Ok(MigrateResponse {
                success: true,
                ir: Some(ir_value),
                source_format: "v4".to_string(),
                target_format: "classic".to_string(),
                warnings,
                error: None,
            })
        }

        // Same format → Just return input
        (LoadedDistribution::Classic(_), false) | (LoadedDistribution::V4(_), true) => {
            let format = if target_v4 { "v4" } else { "classic" };
            Ok(MigrateResponse {
                success: true,
                ir: Some(request.ir),
                source_format: format.to_string(),
                target_format: format.to_string(),
                warnings: vec![],
                error: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrate_info() {
        let migrate = MigrateExtension::default();
        let info = migrate.info();
        assert_eq!(info.id, "migrate");
        assert_eq!(info.extension_type, ExtensionType::Transform);
    }

    #[test]
    fn test_migrate_same_format() {
        let migrate = MigrateExtension::default();

        let request = MigrateRequest {
            ir: serde_json::json!({
                "formatVersion": 1,
                "distribution": ["Library", "Library", ["test"], [], {"modules": {}}]
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

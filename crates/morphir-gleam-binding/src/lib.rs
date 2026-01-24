//! Morphir Gleam Binding Extension
//!
//! This extension provides Gleam language support for Morphir:
//! - Frontend: Parse Gleam source files to Morphir IR
//! - Backend: Generate Gleam code from Morphir IR

use morphir_common::vfs::{OsVfs, Vfs};
use morphir_extension_sdk::prelude::*;
use morphir_ir::naming::{ModuleName, PackageName};
use std::path::PathBuf;

pub mod backend;
pub mod frontend;
pub mod roundtrip;

/// Gleam extension implementing both Frontend and Backend
#[derive(Default)]
pub struct GleamExtension;

impl Extension for GleamExtension {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "morphir-gleam-binding".into(),
            name: "Morphir Gleam Binding".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: Some("Gleam language support for Morphir".into()),
            types: vec![ExtensionType::Frontend, ExtensionType::Backend],
            author: Some("FINOS".into()),
            license: Some("Apache-2.0".into()),
            homepage: Some("https://github.com/finos/morphir-rust".into()),
            min_sdk_version: Some("0.1.0".into()),
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities {
            streaming: false,
            incremental: false,
            cancellation: false,
            progress: false,
            extra: Default::default(),
        }
    }
}

impl Frontend for GleamExtension {
    fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
        host_info!("Compiling {} Gleam source file(s)", request.sources.len());

        let mut ir_modules = Vec::new();
        let mut diagnostics = Vec::new();

        // Determine output directory (from options or default)
        let output_dir = request
            .options
            .get("outputDir")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));

        // Extract package name (from options or default)
        let package_name = request
            .options
            .get("packageName")
            .and_then(|v| v.as_str())
            .map(PackageName::parse)
            .unwrap_or_else(|| PackageName::parse("default-package"));

        for source in &request.sources {
            match frontend::parse_gleam(&source.path, &source.content) {
                Ok(module_ir) => {
                    // Extract module name from path
                    let module_name = ModuleName::parse(
                        &source.path.trim_end_matches(".gleam").replace('\\', "/"),
                    );

                    // Convert to Morphir IR V4 Document Tree format
                    let visitor = frontend::GleamToMorphirVisitor::new(
                        OsVfs,
                        output_dir.clone(),
                        package_name.clone(),
                        module_name,
                    );

                    match visitor.visit_module_v4(&module_ir) {
                        Ok(_) => {
                            // Read format.json as IR representation
                            // output_dir is already .morphir/out/<project>/compile/<language>/
                            let format_json_path = output_dir.join("format.json");
                            let vfs = OsVfs;
                            if vfs.exists(&format_json_path) {
                                match vfs.read_to_string(&format_json_path) {
                                    Ok(format_content) => {
                                        match serde_json::from_str::<serde_json::Value>(
                                            &format_content,
                                        ) {
                                            Ok(ir_json) => {
                                                ir_modules.push(ir_json);
                                            }
                                            Err(e) => {
                                                diagnostics.push(Diagnostic {
                                                    severity: DiagnosticSeverity::Error,
                                                    code: Some("E002".into()),
                                                    message: format!(
                                                        "Failed to parse format.json: {}",
                                                        e
                                                    ),
                                                    location: None,
                                                    related: vec![],
                                                });
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        diagnostics.push(Diagnostic {
                                            severity: DiagnosticSeverity::Error,
                                            code: Some("E003".into()),
                                            message: format!("Failed to read format.json: {}", e),
                                            location: None,
                                            related: vec![],
                                        });
                                    }
                                }
                            } else {
                                // Fallback: serialize ModuleIR directly
                                ir_modules.push(serde_json::to_value(&module_ir)?);
                            }
                        }
                        Err(e) => {
                            diagnostics.push(Diagnostic {
                                severity: DiagnosticSeverity::Error,
                                code: Some("E004".into()),
                                message: format!("Failed to convert to Morphir IR: {}", e),
                                location: None,
                                related: vec![],
                            });
                        }
                    }
                }
                Err(e) => {
                    diagnostics.push(e.to_diagnostic(&source.path, &source.content));
                }
            }
        }

        let success = diagnostics
            .iter()
            .all(|d| d.severity != DiagnosticSeverity::Error);

        Ok(CompileResult {
            success,
            ir: if ir_modules.is_empty() {
                None
            } else {
                Some(serde_json::to_value(&ir_modules)?)
            },
            diagnostics,
        })
    }

    fn supported_languages() -> Vec<String> {
        vec!["gleam".into()]
    }

    fn file_extensions() -> Vec<String> {
        vec![".gleam".into()]
    }
}

impl Backend for GleamExtension {
    fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
        host_info!("Generating Gleam code from IR");

        match backend::generate_gleam(&request.ir, &request.options) {
            Ok(artifacts) => Ok(GenerateResult {
                success: true,
                artifacts,
                diagnostics: vec![],
            }),
            Err(e) => Ok(GenerateResult {
                success: false,
                artifacts: vec![],
                diagnostics: vec![Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    code: Some("G001".into()),
                    message: e.to_string() as String,
                    location: None,
                    related: vec![],
                }],
            }),
        }
    }

    fn target_languages() -> Vec<String> {
        vec!["gleam".into()]
    }
}

// Export the extension
morphir_extension_sdk::export_extension!(GleamExtension);

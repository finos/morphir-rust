//! Morphir Gleam Binding Extension
//!
//! This extension provides Gleam language support for Morphir:
//! - Frontend: Parse Gleam source files to Morphir IR
//! - Backend: Generate Gleam code from Morphir IR

use morphir_common::vfs::OsVfs;
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

        // Check if we should emit parse stage output (default: true)
        let emit_parse_stage = request
            .options
            .get("emitParseStage")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

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
                    // Emit parse stage JSON if enabled
                    if emit_parse_stage
                        && let Err(e) = emit_parse_stage_json(&output_dir, &source.path, &module_ir)
                    {
                        diagnostics.push(Diagnostic {
                            severity: DiagnosticSeverity::Warning,
                            code: Some("W001".into()),
                            message: format!("Failed to emit parse stage output: {}", e),
                            location: None,
                            related: vec![],
                        });
                    }
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
                            // Build format.json in memory without disk I/O
                            // This avoids reading the entire file back after writing it,
                            // which could cause memory issues for large projects
                            let ir_json = visitor.build_format_json();
                            ir_modules.push(ir_json);
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

/// Emit parse stage output as JSON to the output directory
///
/// Writes the parsed ModuleIR to `.morphir/out/<project>/parse/<module>.json`
/// This allows inspection of the intermediate AST before IR conversion.
fn emit_parse_stage_json(
    output_dir: &std::path::Path,
    source_path: &str,
    module_ir: &frontend::ast::ModuleIR,
) -> anyhow::Result<()> {
    use std::fs;

    // Create parse stage output directory
    // output_dir is typically .morphir/out/<project>/compile/<language>/
    // We want to write to .morphir/out/<project>/parse/
    let parse_dir = if let Some(compile_dir) = output_dir.parent() {
        if let Some(project_dir) = compile_dir.parent() {
            project_dir.join("parse")
        } else {
            output_dir.join("parse")
        }
    } else {
        output_dir.join("parse")
    };

    fs::create_dir_all(&parse_dir)?;

    // Derive module filename from source path
    let module_name = source_path
        .trim_end_matches(".gleam")
        .replace(['/', '\\'], "_");
    let output_file = parse_dir.join(format!("{}.json", module_name));

    // Write ModuleIR as pretty-printed JSON
    let json = serde_json::to_string_pretty(module_ir)?;
    fs::write(&output_file, json)?;

    host_info!("Emitted parse stage: {:?}", output_file);
    Ok(())
}

// Export the extension
morphir_extension_sdk::export_extension!(GleamExtension);

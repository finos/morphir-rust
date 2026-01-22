//! Morphir Gleam Binding Extension
//!
//! This extension provides Gleam language support for Morphir:
//! - Frontend: Parse Gleam source files to Morphir IR
//! - Backend: Generate Gleam code from Morphir IR

use morphir_extension_sdk::prelude::*;

mod backend;
mod frontend;

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

        for source in &request.sources {
            match frontend::parse_gleam(&source.path, &source.content) {
                Ok(module_ir) => {
                    ir_modules.push(module_ir);
                }
                Err(e) => {
                    diagnostics.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        code: Some("E001".into()),
                        message: e.to_string() as String,
                        location: Some(SourceLocation {
                            file: source.path.clone(),
                            start_line: 1,
                            start_col: 1,
                            end_line: 1,
                            end_col: 1,
                        }),
                        related: vec![],
                    });
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

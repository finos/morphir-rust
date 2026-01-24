//! Morphir WASM Binding Extension
//!
//! This extension provides WebAssembly code generation for Morphir:
//! - Backend: Generate WASM binary from Morphir IR
//! - Backend: Generate WAT text format from Morphir IR

use morphir_extension_sdk::prelude::*;

mod backend;

/// WASM extension implementing Backend
#[derive(Default)]
pub struct WasmExtension;

impl Extension for WasmExtension {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "morphir-wasm-binding".into(),
            name: "Morphir WASM Backend".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: Some("WebAssembly code generation for Morphir".into()),
            types: vec![ExtensionType::Backend],
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

impl Backend for WasmExtension {
    fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
        host_info!("Generating WASM from IR");

        let emit_wat = request
            .options
            .get("emit_wat")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        match backend::generate_wasm(&request.ir, &request.options) {
            Ok(mut artifacts) => {
                // Optionally generate WAT as well
                if emit_wat
                    && let Ok(wat_artifacts) = backend::generate_wat(&request.ir, &request.options)
                {
                    artifacts.extend(wat_artifacts);
                }

                Ok(GenerateResult {
                    success: true,
                    artifacts,
                    diagnostics: vec![],
                })
            }
            Err(e) => Ok(GenerateResult {
                success: false,
                artifacts: vec![],
                diagnostics: vec![Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    code: Some("W001".into()),
                    message: e.to_string() as String,
                    location: None,
                    related: vec![],
                }],
            }),
        }
    }

    fn target_languages() -> Vec<String> {
        vec!["wasm".into(), "wat".into()]
    }
}

// Export the extension
morphir_extension_sdk::export_extension!(WasmExtension);

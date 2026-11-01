//! Rust type and pure-function frontend and backend for Morphir IR v3 and v4.
//! Explicit native and external function bindings are extracted into v4 value definitions.
//!
//! The extension speaks MEP `0.1` and MEP `0.2.0-draft.1`. On the second line it also serves the
//! model operations `morphir.frontend.compileModel` and `morphir.backend.generateModel` for the
//! tuple `morphir-ir` / `morphir-json` / envelope `0.1.0-draft.1` / IR `4.0.0`.
//!
//! ```
//! use morphir_extension_sdk::{Extension, NativeExtension};
//! use morphir_rust_binding::RustExtension;
//! let extension = NativeExtension::frontend_backend(RustExtension).unwrap();
//! assert_eq!(RustExtension::info().id, "morphir-rust");
//! ```

mod backend;
mod frontend;
mod functions;
mod model_transport;
mod patterns;
mod values;

use morphir_extension_sdk::prelude::*;

/// Stateless Rust language extension, usable natively or as a WASM guest.
#[derive(Default)]
pub struct RustExtension;

impl Extension for RustExtension {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "morphir-rust".into(),
            name: "Morphir Rust".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            types: vec![ExtensionType::Frontend, ExtensionType::Backend],
            description: Some(
                "Rust types, conditionals and pattern matching for IR v3/v4, with explicit v4 bindings"
                    .into(),
            ),
            license: Some("Apache-2.0".into()),
            ..Default::default()
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities {
            frontend: Some(FrontendCapability {
                languages: vec![LanguageCapability {
                    id: "rust".into(),
                    file_extensions: vec![".rs".into()],
                }],
                ir_versions: vec!["3".into(), "4".into()],
                compile: true,
                ..Default::default()
            }),
            backend: Some(BackendCapability {
                targets: vec!["rust".into()],
                ir_versions: vec!["3".into(), "4".into()],
                generate: true,
            }),
            model_transport: Some(model_transport::support()),
            ..Default::default()
        }
    }
}

impl Frontend for RustExtension {
    fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
        Ok(
            frontend::compile(&request).unwrap_or_else(|diagnostic| CompileResult {
                success: false,
                ir_version: None,
                ir: None,
                diagnostics: vec![diagnostic],
                modules: vec![],
                module_results: vec![],
                context_digest: None,
            }),
        )
    }

    fn compile_model(
        &self,
        request: morphir_extension_sdk::model::CompileModelRequest,
    ) -> Result<morphir_extension_sdk::model::CompileModelResult> {
        model_transport::compile_model(request)
    }

    fn supported_languages() -> Vec<String> {
        vec!["rust".into()]
    }
    fn file_extensions() -> Vec<String> {
        vec![".rs".into()]
    }
}

impl Backend for RustExtension {
    fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
        Ok(match backend::generate(&request) {
            Ok(result) => result,
            Err(diagnostic) => GenerateResult {
                success: false,
                artifacts: vec![],
                diagnostics: vec![diagnostic],
            },
        })
    }

    fn generate_model(
        &self,
        request: morphir_extension_sdk::model::GenerateModelRequest,
    ) -> Result<GenerateResult> {
        model_transport::generate_model(request)
    }

    fn target_languages() -> Vec<String> {
        vec!["rust".into()]
    }
}

morphir_extension_sdk::export_extension!(RustExtension, frontend, backend);

type Outcome<T> = std::result::Result<T, Diagnostic>;

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        severity: DiagnosticSeverity::Error,
        code: Some(code.into()),
        message: message.into(),
        location: None,
        related: vec![],
    }
}

//! Rust type and pure-function frontend and backend for Morphir IR v3 and v4.
//! Explicit native and external function bindings are extracted into v4 value definitions.
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
            }),
        )
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

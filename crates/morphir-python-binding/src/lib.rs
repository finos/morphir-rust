//! Python ADT extension for Morphir.
//!
//! Source is parsed statically, never imported or executed.
//! See the crate README for the supported Python subset.
//!
//! ```
//! use morphir_extension_sdk::{Extension, NativeExtension};
//! use morphir_python_binding::PythonExtension;
//! let extension = NativeExtension::frontend_backend(PythonExtension).unwrap();
//! assert_eq!(PythonExtension::info().id, "morphir-python");
//! ```

mod backend;
mod frontend;
mod names;

use morphir_extension_sdk::prelude::*;

/// Stateless Python ADT frontend and backend, usable natively or as a WASM guest.
#[derive(Default)]
pub struct PythonExtension;

impl Extension for PythonExtension {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "morphir-python".into(),
            name: "Morphir Python".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            types: vec![ExtensionType::Frontend, ExtensionType::Backend],
            description: Some("Python algebraic data types".into()),
            license: Some("Apache-2.0".into()),
            ..Default::default()
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities {
            frontend: Some(FrontendCapability {
                languages: vec![LanguageCapability {
                    id: "python".into(),
                    file_extensions: vec![".py".into()],
                }],
                ir_versions: vec!["4".into()],
                compile: true,
                ..Default::default()
            }),
            backend: Some(BackendCapability {
                targets: vec!["python".into()],
                ir_versions: vec!["4".into()],
                generate: true,
            }),
            ..Default::default()
        }
    }
}

impl Frontend for PythonExtension {
    fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
        let diagnostics = if frontend::parse_stage_requested(&request) {
            vec![Diagnostic {
                severity: DiagnosticSeverity::Warning,
                ..error(
                    "PY006",
                    "Python parse-stage output is not implemented; returning compiled IR only",
                )
            }]
        } else {
            vec![]
        };
        Ok(match frontend::compile(&request) {
            Ok((ir, module)) => CompileResult {
                success: true,
                ir_version: Some("4".into()),
                ir: Some(ir),
                modules: vec![module],
                diagnostics,
            },
            Err(diagnostic) => CompileResult {
                success: false,
                ir_version: None,
                ir: None,
                modules: vec![],
                diagnostics: vec![diagnostic],
            },
        })
    }
    fn supported_languages() -> Vec<String> {
        vec!["python".into()]
    }
    fn file_extensions() -> Vec<String> {
        vec![".py".into()]
    }
}

impl Backend for PythonExtension {
    fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
        Ok(match backend::generate(&request) {
            Ok(artifact) => GenerateResult {
                success: true,
                artifacts: vec![artifact],
                diagnostics: vec![],
            },
            Err(diagnostic) => GenerateResult {
                success: false,
                artifacts: vec![],
                diagnostics: vec![diagnostic],
            },
        })
    }
    fn target_languages() -> Vec<String> {
        vec!["python".into()]
    }
}

morphir_extension_sdk::export_extension!(PythonExtension, frontend, backend);

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

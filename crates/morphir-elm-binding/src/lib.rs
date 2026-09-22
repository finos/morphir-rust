//! Elm type frontend and backend for Morphir IR v3 and v4.
//!
//! ```
//! use morphir_extension_sdk::{Extension, NativeExtension};
//! use morphir_elm_binding::ElmExtension;
//! let extension = NativeExtension::builder(ElmExtension)
//!     .with_frontend()
//!     .with_backend()
//!     .with_workspace()
//!     .finish()
//!     .unwrap();
//! assert_eq!(ElmExtension::info().id, "morphir-elm-native");
//! ```
#![warn(missing_docs)]

pub mod ast;
pub mod backend;
pub mod digest;
pub mod frontend;
pub mod incremental;
pub mod names;
pub mod prelude;
pub mod resolved;
pub mod span;
mod workspace;

use morphir_extension_sdk::prelude::*;

/// The extension identifier the daemon registers this binding under.
pub const EXTENSION_ID: &str = "morphir-elm-native";

/// The stateless Elm language extension, usable natively or as a WASM guest.
#[derive(Default)]
pub struct ElmExtension;

impl Extension for ElmExtension {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: EXTENSION_ID.into(),
            name: "Morphir Elm (native)".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            types: vec![
                ExtensionType::Frontend,
                ExtensionType::Backend,
                ExtensionType::Workspace,
            ],
            description: Some(
                "Elm type declarations for Morphir IR 3 and 4, with incremental compilation".into(),
            ),
            license: Some("Apache-2.0".into()),
            ..Default::default()
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities {
            frontend: Some(FrontendCapability {
                languages: vec![LanguageCapability {
                    id: "elm".into(),
                    file_extensions: vec![".elm".into()],
                }],
                ir_versions: vec!["3".into(), "4".into()],
                compile: true,
                incremental: true,
                fragments: false,
                multi_document: true,
            }),
            backend: Some(BackendCapability {
                targets: vec!["elm".into()],
                ir_versions: vec!["3".into(), "4".into()],
                generate: true,
            }),
            workspace: Some(WorkspaceCapability {
                protocol_versions: vec![morphir_workspace::WORKSPACE_DISCOVERY_PROTOCOL],
                discover: true,
            }),
            incremental: true,
            ..Default::default()
        }
    }
}

impl Frontend for ElmExtension {
    fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
        Ok(frontend::compile::compile(request))
    }

    fn supported_languages() -> Vec<String> {
        vec!["elm".into()]
    }

    fn file_extensions() -> Vec<String> {
        vec![".elm".into()]
    }
}

impl Backend for ElmExtension {
    fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
        Ok(backend::generate(request))
    }

    fn target_languages() -> Vec<String> {
        vec!["elm".into()]
    }
}

// The guest exports this expands to are reached by symbol name from the WASM
// host, not by any Rust caller, and the macro writes them without doc
// comments; the module is only here to carry the exemption.
// The import is only read by the guest exports, which exist on wasm32 alone.
#[allow(missing_docs, unused_imports)]
mod guest {
    use super::ElmExtension;

    morphir_extension_sdk::export_extension!(ElmExtension, frontend, backend, workspace);
}

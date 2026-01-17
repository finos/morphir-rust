//! Morphir Extension SDK
//!
//! This crate provides the SDK for building Morphir extensions as WASM plugins.
//! Extensions communicate with the host daemon via JSON-RPC 2.0 payloads.
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use morphir_extension_sdk::prelude::*;
//!
//! #[derive(Default)]
//! struct MyExtension;
//!
//! impl Extension for MyExtension {
//!     fn info() -> ExtensionInfo {
//!         ExtensionInfo {
//!             id: "my-extension".into(),
//!             name: "My Extension".into(),
//!             version: env!("CARGO_PKG_VERSION").into(),
//!             types: vec![ExtensionType::Frontend],
//!             ..Default::default()
//!         }
//!     }
//! }
//!
//! impl Frontend for MyExtension {
//!     fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
//!         // Compile source files to IR
//!         Ok(CompileResult {
//!             success: true,
//!             ir: Some(serde_json::json!({})),
//!             diagnostics: vec![],
//!         })
//!     }
//!
//!     fn supported_languages() -> Vec<String> {
//!         vec!["my-lang".into()]
//!     }
//!
//!     fn file_extensions() -> Vec<String> {
//!         vec![".ml".into()]
//!     }
//! }
//!
//! morphir_extension_sdk::export_extension!(MyExtension);
//! ```
//!
//! # Extension Types
//!
//! - **Frontend**: Compiles source code to Morphir IR
//! - **Backend**: Generates code from Morphir IR
//! - **Validator**: Validates IR and produces diagnostics
//! - **Transform**: Transforms IR to IR

pub mod error;
pub mod host;
pub mod prelude;
pub mod protocol;
pub mod traits;
pub mod types;

// Re-exports
pub use error::{ExtensionError, Result};
pub use traits::{Backend, Extension, Frontend, Transform, Validator};
pub use types::*;

/// Export an extension implementation with JSON-RPC dispatch
///
/// This macro generates the necessary WASM exports for your extension:
/// - `morphir_extension_info`: Returns extension metadata
/// - `handle`: Main JSON-RPC request handler
///
/// # Example
///
/// ```rust,ignore
/// use morphir_extension_sdk::prelude::*;
///
/// #[derive(Default)]
/// struct MyExtension;
///
/// impl Extension for MyExtension {
///     fn info() -> ExtensionInfo { /* ... */ }
/// }
///
/// morphir_extension_sdk::export_extension!(MyExtension);
/// ```
#[macro_export]
macro_rules! export_extension {
    ($impl:ty) => {
        use $crate::extism_pdk::*;

        /// Extension info function (required by host)
        #[plugin_fn]
        pub fn morphir_extension_info() -> FnResult<Json<$crate::ExtensionInfo>> {
            Ok(Json(<$impl as $crate::Extension>::info()))
        }

        /// Extension capabilities function
        #[plugin_fn]
        pub fn morphir_extension_capabilities() -> FnResult<Json<$crate::ExtensionCapabilities>> {
            Ok(Json(<$impl as $crate::Extension>::capabilities()))
        }

        /// Main JSON-RPC handler
        #[plugin_fn]
        pub fn handle(
            Json(request): Json<$crate::protocol::ExtensionRequest>,
        ) -> FnResult<Json<$crate::protocol::ExtensionResponse>> {
            let result = $crate::__dispatch_request::<$impl>(&request);
            Ok(Json(result))
        }
    };
}

/// Internal dispatch function used by export_extension! macro
#[doc(hidden)]
pub fn __dispatch_request<E: Extension + Default>(
    request: &protocol::ExtensionRequest,
) -> protocol::ExtensionResponse {
    use protocol::methods;

    let result = match request.method.as_str() {
        methods::INFO => serde_json::to_value(E::info()),

        methods::CAPABILITIES => serde_json::to_value(E::capabilities()),

        methods::COMPILE => {
            dispatch_compile::<E>(request)
        }

        methods::GENERATE => {
            dispatch_generate::<E>(request)
        }

        methods::VALIDATE => {
            dispatch_validate::<E>(request)
        }

        methods::TRANSFORM => {
            dispatch_transform::<E>(request)
        }

        method => {
            return protocol::ExtensionResponse::error(
                request.id,
                protocol::RpcError::method_not_found(method),
            );
        }
    };

    match result {
        Ok(value) => protocol::ExtensionResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(value),
            error: None,
            id: request.id,
        },
        Err(e) => protocol::ExtensionResponse {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(protocol::RpcError::internal_error(e.to_string())),
            id: request.id,
        },
    }
}

#[doc(hidden)]
fn dispatch_compile<E: Extension + Default>(
    request: &protocol::ExtensionRequest,
) -> std::result::Result<serde_json::Value, serde_json::Error> {
    // This uses trait bounds, so we need to handle the case where E doesn't implement Frontend
    // We'll do this by trying to deserialize and call, but the actual implementation
    // will be handled by the concrete type

    // For now, return method not found if the extension doesn't implement the trait
    // The actual dispatching will be done by the extension's concrete implementation
    let _params: types::CompileRequest = serde_json::from_value(request.params.clone())?;

    // This is a placeholder - the actual implementation should be provided
    // by extensions that implement Frontend
    let result = types::CompileResult {
        success: false,
        ir: None,
        diagnostics: vec![types::Diagnostic {
            severity: types::DiagnosticSeverity::Error,
            code: None,
            message: "Frontend not implemented".to_string(),
            location: None,
            related: vec![],
        }],
    };

    serde_json::to_value(result)
}

#[doc(hidden)]
fn dispatch_generate<E: Extension + Default>(
    request: &protocol::ExtensionRequest,
) -> std::result::Result<serde_json::Value, serde_json::Error> {
    let _params: types::GenerateRequest = serde_json::from_value(request.params.clone())?;

    let result = types::GenerateResult {
        success: false,
        artifacts: vec![],
        diagnostics: vec![types::Diagnostic {
            severity: types::DiagnosticSeverity::Error,
            code: None,
            message: "Backend not implemented".to_string(),
            location: None,
            related: vec![],
        }],
    };

    serde_json::to_value(result)
}

#[doc(hidden)]
fn dispatch_validate<E: Extension + Default>(
    request: &protocol::ExtensionRequest,
) -> std::result::Result<serde_json::Value, serde_json::Error> {
    let _params: types::ValidateRequest = serde_json::from_value(request.params.clone())?;

    let result = types::ValidateResult {
        valid: false,
        diagnostics: vec![types::Diagnostic {
            severity: types::DiagnosticSeverity::Error,
            code: None,
            message: "Validator not implemented".to_string(),
            location: None,
            related: vec![],
        }],
    };

    serde_json::to_value(result)
}

#[doc(hidden)]
fn dispatch_transform<E: Extension + Default>(
    request: &protocol::ExtensionRequest,
) -> std::result::Result<serde_json::Value, serde_json::Error> {
    let _params: types::TransformRequest = serde_json::from_value(request.params.clone())?;

    let result = types::TransformResult {
        success: false,
        ir: None,
        diagnostics: vec![types::Diagnostic {
            severity: types::DiagnosticSeverity::Error,
            code: None,
            message: "Transform not implemented".to_string(),
            location: None,
            related: vec![],
        }],
    };

    serde_json::to_value(result)
}

// Re-export extism_pdk for use in macro
#[doc(hidden)]
pub use extism_pdk;

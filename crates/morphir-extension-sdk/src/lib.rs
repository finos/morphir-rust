//! Morphir Extension SDK
//!
//! This crate provides the SDK for building Morphir extensions as WASM plugins.
//! Extensions communicate with the host daemon via JSON-RPC 2.0 payloads.
//!
//! # Platform boundary
//!
//! Protocol types and extension traits compile on native and WASM targets. The
//! Extism PDK, guest exports, and imported host functions compile only for
//! `wasm32`. Native hosts must use the Extism runtime crate and must not link
//! guest PDK imports.
//!
//! # Quick Start
//!
//! ```rust
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
//!
//!     fn capabilities() -> ExtensionCapabilities {
//!         ExtensionCapabilities {
//!             frontend: Some(FrontendCapability {
//!                 languages: vec![LanguageCapability {
//!                     id: "my-lang".into(),
//!                     file_extensions: vec![".ml".into()],
//!                 }],
//!                 ir_versions: vec!["3".into()],
//!                 compile: true,
//!                 incremental: false,
//!                 fragments: false,
//!             }),
//!             ..ExtensionCapabilities::default()
//!         }
//!     }
//! }
//!
//! impl Frontend for MyExtension {
//!     fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
//!         Ok(CompileResult {
//!             success: true,
//!             ir_version: Some(request.options.ir_version),
//!             ir: Some(serde_json::json!({})),
//!             diagnostics: vec![],
//!             modules: request.package.exposed_modules,
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
//! let request = CompileRequest {
//!     language_id: "my-lang".into(),
//!     documents: vec![SourceDocument {
//!         uri: "file:///workspace/Example.ml".into(),
//!         language_id: "my-lang".into(),
//!         version: 1,
//!         text: "module Example".into(),
//!     }],
//!     package: CompilePackage {
//!         name: "local/example".into(),
//!         exposed_modules: vec!["Example".into()],
//!     },
//!     dependencies: vec![],
//!     options: CompileOptions {
//!         types_only: false,
//!         ir_version: "3".into(),
//!         extra: Default::default(),
//!     },
//! };
//! let result = MyExtension.compile(request).expect("frontend should compile");
//! assert!(result.success);
//!
//! morphir_extension_sdk::export_extension!(MyExtension, frontend);
//! ```

pub mod error;
#[cfg(target_arch = "wasm32")]
pub mod host;
pub mod prelude;
pub mod protocol;
pub mod traits;
pub mod types;

// Re-exports
pub use error::{ExtensionError, Result};
pub use traits::{Backend, Extension, Frontend, Transform, Validator, Workspace};
pub use types::*;

/// Export an extension implementation with JSON-RPC dispatch
///
/// This macro generates the necessary WASM exports for your extension:
/// - `morphir_extension_info`: Returns extension metadata
/// - `handle`: Main JSON-RPC request handler
#[macro_export]
macro_rules! export_extension {
    ($impl:ty, $($capability:ident),+ $(,)?) => {
        #[cfg(target_arch = "wasm32")]
        use $crate::extism_pdk as extism_pdk;
        #[cfg(target_arch = "wasm32")]
        use extism_pdk::*;

        /// Extension info function (required by host)
        #[cfg(target_arch = "wasm32")]
        #[plugin_fn]
        pub fn morphir_extension_info() -> FnResult<Json<$crate::ExtensionInfo>> {
            let mut declared_types = Vec::new();
            $crate::__push_extension_types!(declared_types, $($capability),+);
            Ok(Json($crate::__extension_info::<$impl>(&declared_types)))
        }

        /// Extension capabilities function
        #[cfg(target_arch = "wasm32")]
        #[plugin_fn]
        pub fn morphir_extension_capabilities() -> FnResult<Json<$crate::ExtensionCapabilities>> {
            Ok(Json(<$impl as $crate::Extension>::capabilities()))
        }

        /// Main JSON-RPC handler
        #[cfg(target_arch = "wasm32")]
        #[plugin_fn]
        pub fn handle(
            Json(request): Json<$crate::protocol::ExtensionRequest>,
        ) -> FnResult<Json<$crate::protocol::ExtensionResponse>> {
            let mut dispatchers: Vec<$crate::DispatchFn<$impl>> = Vec::new();
            let mut declared_types = Vec::new();
            $crate::__push_extension_dispatchers!(dispatchers, $impl, $($capability),+);
            $crate::__push_extension_types!(declared_types, $($capability),+);
            let result = $crate::__dispatch_request::<$impl>(
                &request,
                &dispatchers,
                &declared_types,
            );
            Ok(Json(result))
        }
    };
}

/// Add declared extension capability types to generated guest metadata.
#[doc(hidden)]
#[macro_export]
macro_rules! __push_extension_types {
    ($types:ident, backend $(, $remaining:ident)*) => {
        $types.push($crate::ExtensionType::Backend);
        $crate::__push_extension_types!($types $(, $remaining)*);
    };
    ($types:ident, frontend $(, $remaining:ident)*) => {
        $types.push($crate::ExtensionType::Frontend);
        $crate::__push_extension_types!($types $(, $remaining)*);
    };
    ($types:ident, validator $(, $remaining:ident)*) => {
        $types.push($crate::ExtensionType::Validator);
        $crate::__push_extension_types!($types $(, $remaining)*);
    };
    ($types:ident, transform $(, $remaining:ident)*) => {
        $types.push($crate::ExtensionType::Transform);
        $crate::__push_extension_types!($types $(, $remaining)*);
    };
    ($types:ident, workspace $(, $remaining:ident)*) => {
        $types.push($crate::ExtensionType::Workspace);
        $crate::__push_extension_types!($types $(, $remaining)*);
    };
    ($types:ident) => {};
}

/// Add declared extension capabilities to the generated guest dispatcher.
#[doc(hidden)]
#[macro_export]
macro_rules! __push_extension_dispatchers {
    ($dispatchers:ident, $impl:ty, backend $(, $remaining:ident)*) => {
        $dispatchers.push($crate::__dispatch_backend::<$impl>);
        $crate::__push_extension_dispatchers!($dispatchers, $impl $(, $remaining)*);
    };
    ($dispatchers:ident, $impl:ty, frontend $(, $remaining:ident)*) => {
        $dispatchers.push($crate::__dispatch_frontend::<$impl>);
        $crate::__push_extension_dispatchers!($dispatchers, $impl $(, $remaining)*);
    };
    ($dispatchers:ident, $impl:ty, validator $(, $remaining:ident)*) => {
        $dispatchers.push($crate::__dispatch_validator::<$impl>);
        $crate::__push_extension_dispatchers!($dispatchers, $impl $(, $remaining)*);
    };
    ($dispatchers:ident, $impl:ty, transform $(, $remaining:ident)*) => {
        $dispatchers.push($crate::__dispatch_transform::<$impl>);
        $crate::__push_extension_dispatchers!($dispatchers, $impl $(, $remaining)*);
    };
    ($dispatchers:ident, $impl:ty, workspace $(, $remaining:ident)*) => {
        $dispatchers.push($crate::__dispatch_workspace::<$impl>);
        $crate::__push_extension_dispatchers!($dispatchers, $impl $(, $remaining)*);
    };
    ($dispatchers:ident, $impl:ty) => {};
}

// Native builds exercise extension implementations without importing guest PDK symbols.
#[cfg(not(target_arch = "wasm32"))]
#[macro_export]
macro_rules! host_debug {
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }};
}

#[cfg(not(target_arch = "wasm32"))]
#[macro_export]
macro_rules! host_info {
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }};
}

#[cfg(not(target_arch = "wasm32"))]
#[macro_export]
macro_rules! host_warn {
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }};
}

#[cfg(not(target_arch = "wasm32"))]
#[macro_export]
macro_rules! host_error {
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }};
}

/// A capability-specific request dispatcher used by [`export_extension!`].
#[doc(hidden)]
pub type DispatchFn<E> = fn(
    &E,
    &protocol::ExtensionRequest,
) -> Option<std::result::Result<serde_json::Value, ExtensionError>>;

/// Return extension metadata aligned with the macro-declared capabilities.
#[doc(hidden)]
pub fn __extension_info<E: Extension>(declared_types: &[ExtensionType]) -> ExtensionInfo {
    let mut info = E::info();
    info.types = declared_types.to_vec();
    info
}

/// Internal dispatch function used by export_extension! macro.
#[doc(hidden)]
pub fn __dispatch_request<E: Extension + Default>(
    request: &protocol::ExtensionRequest,
    dispatchers: &[DispatchFn<E>],
    declared_types: &[ExtensionType],
) -> protocol::ExtensionResponse {
    use protocol::methods;

    let result = match request.method.as_str() {
        methods::INITIALIZE => dispatch_initialize::<E>(request, declared_types),
        methods::PING => Ok(serde_json::json!({ "ok": true })),
        methods::INFO => serde_json::to_value(__extension_info::<E>(declared_types))
            .map_err(ExtensionError::from),
        methods::CAPABILITIES => {
            serde_json::to_value(E::capabilities()).map_err(ExtensionError::from)
        }
        methods::SHUTDOWN => Ok(serde_json::json!({})),
        _ => {
            let extension = E::default();
            let Some(result) = dispatchers
                .iter()
                .find_map(|dispatch| dispatch(&extension, request))
            else {
                return protocol::ExtensionResponse::error(
                    request.id,
                    protocol::RpcError::method_not_found(&request.method),
                );
            };
            result
        }
    };

    match result {
        Ok(value) => protocol::ExtensionResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(value),
            error: None,
            id: request.id,
        },
        Err(error) => protocol::ExtensionResponse::from_extension_error(request.id, &error),
    }
}

fn dispatch_initialize<E: Extension>(
    request: &protocol::ExtensionRequest,
    declared_types: &[ExtensionType],
) -> std::result::Result<serde_json::Value, ExtensionError> {
    let params: protocol::InitializeParams = serde_json::from_value(request.params.clone())
        .map_err(|error| ExtensionError::InvalidParams(error.to_string()))?;
    if !params
        .protocol_versions
        .iter()
        .any(|version| version == protocol::MEP_VERSION)
    {
        return Err(ExtensionError::ProtocolVersionMismatch {
            host_versions: params.protocol_versions,
            extension_versions: vec![protocol::MEP_VERSION.to_string()],
        });
    }

    serde_json::to_value(protocol::InitializeResult {
        protocol_version: protocol::MEP_VERSION.to_string(),
        extension: __extension_info::<E>(declared_types),
        capabilities: E::capabilities(),
    })
    .map_err(ExtensionError::from)
}

#[doc(hidden)]
pub fn __dispatch_frontend<E: Frontend>(
    extension: &E,
    request: &protocol::ExtensionRequest,
) -> Option<std::result::Result<serde_json::Value, ExtensionError>> {
    (request.method == protocol::methods::COMPILE).then(|| {
        let params: types::CompileRequest = serde_json::from_value(request.params.clone())
            .map_err(|error| ExtensionError::InvalidParams(error.to_string()))?;
        serde_json::to_value(extension.compile(params)?).map_err(ExtensionError::from)
    })
}

#[doc(hidden)]
pub fn __dispatch_backend<E: Backend>(
    extension: &E,
    request: &protocol::ExtensionRequest,
) -> Option<std::result::Result<serde_json::Value, ExtensionError>> {
    (request.method == protocol::methods::GENERATE).then(|| {
        let params: types::GenerateRequest = serde_json::from_value(request.params.clone())
            .map_err(|error| ExtensionError::InvalidParams(error.to_string()))?;
        serde_json::to_value(extension.generate(params)?).map_err(ExtensionError::from)
    })
}

#[doc(hidden)]
pub fn __dispatch_validator<E: Validator>(
    extension: &E,
    request: &protocol::ExtensionRequest,
) -> Option<std::result::Result<serde_json::Value, ExtensionError>> {
    (request.method == protocol::methods::VALIDATE).then(|| {
        let params: types::ValidateRequest = serde_json::from_value(request.params.clone())
            .map_err(|error| ExtensionError::InvalidParams(error.to_string()))?;
        serde_json::to_value(extension.validate(params)?).map_err(ExtensionError::from)
    })
}

#[doc(hidden)]
pub fn __dispatch_transform<E: Transform>(
    extension: &E,
    request: &protocol::ExtensionRequest,
) -> Option<std::result::Result<serde_json::Value, ExtensionError>> {
    (request.method == protocol::methods::TRANSFORM).then(|| {
        let params: types::TransformRequest = serde_json::from_value(request.params.clone())
            .map_err(|error| ExtensionError::InvalidParams(error.to_string()))?;
        serde_json::to_value(extension.transform(params)?).map_err(ExtensionError::from)
    })
}

#[doc(hidden)]
pub fn __dispatch_workspace<E: Workspace>(
    extension: &E,
    request: &protocol::ExtensionRequest,
) -> Option<std::result::Result<serde_json::Value, ExtensionError>> {
    (request.method == protocol::methods::WORKSPACE_DISCOVER).then(|| {
        let params: morphir_workspace::DiscoveryRequest =
            serde_json::from_value(request.params.clone())
                .map_err(|error| ExtensionError::InvalidParams(error.to_string()))?;
        serde_json::to_value(extension.discover(params)?).map_err(ExtensionError::from)
    })
}

// Re-export extism_pdk for use in macro
#[doc(hidden)]
#[cfg(target_arch = "wasm32")]
pub use extism_pdk;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ExtensionRequest, methods};
    use morphir_workspace::{
        DiscoveryRequest, DiscoveryResponse, FileEntry, FileTree, RelativePath,
        WORKSPACE_DISCOVERY_PROTOCOL,
    };
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct RecordingBackend;

    impl Extension for RecordingBackend {
        fn info() -> ExtensionInfo {
            ExtensionInfo {
                id: "recording-backend".into(),
                name: "Recording backend".into(),
                types: vec![],
                ..ExtensionInfo::default()
            }
        }
    }

    impl Backend for RecordingBackend {
        fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
            Ok(GenerateResult {
                success: true,
                artifacts: vec![Artifact {
                    path: "observed.json".into(),
                    content: request.ir.to_string(),
                    binary: false,
                }],
                diagnostics: vec![],
            })
        }

        fn target_languages() -> Vec<String> {
            vec!["recording".into()]
        }
    }

    #[derive(Default)]
    struct RecordingWorkspace;

    impl Extension for RecordingWorkspace {
        fn info() -> ExtensionInfo {
            ExtensionInfo {
                id: "recording-workspace".into(),
                name: "Recording workspace".into(),
                version: "1.0.0".into(),
                types: vec![],
                ..ExtensionInfo::default()
            }
        }

        fn capabilities() -> ExtensionCapabilities {
            ExtensionCapabilities {
                workspace: Some(WorkspaceCapability {
                    protocol_versions: vec![WORKSPACE_DISCOVERY_PROTOCOL],
                    discover: true,
                }),
                ..ExtensionCapabilities::default()
            }
        }
    }

    impl Workspace for RecordingWorkspace {
        fn discover(&self, request: DiscoveryRequest) -> Result<DiscoveryResponse> {
            Ok(morphir_workspace::discover(request))
        }
    }

    fn a_workspace_request() -> DiscoveryRequest {
        DiscoveryRequest {
            protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
            development_root: FileTree {
                entries: BTreeMap::from([
                    (RelativePath::root(), FileEntry::Directory),
                    (
                        RelativePath::parse("morphir.toml").unwrap(),
                        FileEntry::File {
                            text: "[project]\nname = \"acme/orders\"\n".into(),
                        },
                    ),
                ]),
            },
            morphir_home: None,
            system_config: None,
            environment: BTreeMap::new(),
            cli_overlay: serde_json::json!({}),
        }
    }

    fn a_generate_request() -> ExtensionRequest {
        ExtensionRequest::new(
            methods::GENERATE,
            GenerateRequest {
                ir: serde_json::json!({ "observed": true }),
                target: "recording".into(),
                options: Default::default(),
            },
            7,
        )
        .expect("the request fixture should serialize")
    }

    fn an_initialize_request(protocol_versions: Vec<&str>) -> ExtensionRequest {
        ExtensionRequest::new(
            methods::INITIALIZE,
            protocol::InitializeParams {
                protocol_versions: protocol_versions.into_iter().map(str::to_string).collect(),
                host: protocol::PeerInfo {
                    name: "conformance-host".into(),
                    version: "0.1.0".into(),
                },
            },
            1,
        )
        .expect("the initialization fixture should serialize")
    }

    #[test]
    fn backend_requests_reach_the_backend_implementation() {
        let response = __dispatch_request::<RecordingBackend>(
            &a_generate_request(),
            &[__dispatch_backend::<RecordingBackend>],
            &[ExtensionType::Backend],
        );
        let result: GenerateResult = serde_json::from_value(
            response
                .result
                .expect("backend dispatch should return a successful result"),
        )
        .expect("backend dispatch should return a GenerateResult");

        assert!(result.success);
        assert_eq!(result.artifacts.len(), 1);
        assert_eq!(result.artifacts[0].path, "observed.json");
        assert_eq!(result.artifacts[0].content, r#"{"observed":true}"#);
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn workspace_requests_reach_the_workspace_implementation() {
        let response = __dispatch_request::<RecordingWorkspace>(
            &ExtensionRequest::new(methods::WORKSPACE_DISCOVER, a_workspace_request(), 8)
                .expect("the workspace request should serialize"),
            &[__dispatch_workspace::<RecordingWorkspace>],
            &[ExtensionType::Workspace],
        );
        let result: DiscoveryResponse = serde_json::from_value(
            response
                .result
                .expect("workspace dispatch should return a discovery response"),
        )
        .expect("workspace dispatch should return a typed response");

        let snapshot = result
            .into_result()
            .expect("the workspace fixture should discover");
        assert_eq!(snapshot.projects[0].name, "acme/orders");
    }

    #[test]
    fn malformed_workspace_parameters_are_invalid_params() {
        let request = ExtensionRequest::new(
            methods::WORKSPACE_DISCOVER,
            serde_json::json!({ "protocolVersion": "one" }),
            9,
        )
        .expect("the malformed request should still be valid JSON-RPC");
        let response = __dispatch_request::<RecordingWorkspace>(
            &request,
            &[__dispatch_workspace::<RecordingWorkspace>],
            &[ExtensionType::Workspace],
        );

        assert_eq!(
            response
                .error
                .expect("workspace parameters should fail")
                .code,
            protocol::error_codes::INVALID_PARAMS
        );
    }

    #[test]
    fn initialization_reports_the_workspace_contract() {
        let response = __dispatch_request::<RecordingWorkspace>(
            &an_initialize_request(vec!["0.1"]),
            &[__dispatch_workspace::<RecordingWorkspace>],
            &[ExtensionType::Workspace],
        );
        let result: protocol::InitializeResult =
            serde_json::from_value(response.result.expect("workspace peers should initialize"))
                .expect("initialization should return the negotiated session");

        assert_eq!(result.extension.types, vec![ExtensionType::Workspace]);
        assert_eq!(
            result.capabilities.workspace,
            Some(WorkspaceCapability {
                protocol_versions: vec![WORKSPACE_DISCOVERY_PROTOCOL],
                discover: true,
            })
        );
    }

    #[test]
    fn initialization_negotiates_mep_0_1_and_reports_the_extension() {
        let response = __dispatch_request::<RecordingBackend>(
            &an_initialize_request(vec!["0.1"]),
            &[__dispatch_backend::<RecordingBackend>],
            &[ExtensionType::Backend],
        );
        let result: protocol::InitializeResult =
            serde_json::from_value(response.result.expect("compatible peers should initialize"))
                .expect("initialization should return the negotiated session");

        assert_eq!(result.protocol_version, protocol::MEP_VERSION);
        assert_eq!(result.extension.id, "recording-backend");
        assert_eq!(result.extension.types, vec![ExtensionType::Backend]);
    }

    #[test]
    fn initialization_rejects_incompatible_protocol_versions() {
        let response = __dispatch_request::<RecordingBackend>(
            &an_initialize_request(vec!["9.0"]),
            &[__dispatch_backend::<RecordingBackend>],
            &[ExtensionType::Backend],
        );

        assert!(response.result.is_none());
        assert_eq!(
            response.error.expect("initialization should fail").code,
            protocol::error_codes::PROTOCOL_VERSION_MISMATCH
        );
    }

    #[test]
    fn malformed_method_parameters_are_invalid_params() {
        let request = ExtensionRequest::new(methods::GENERATE, serde_json::json!({}), 9)
            .expect("the malformed request should still be valid JSON-RPC");
        let response = __dispatch_request::<RecordingBackend>(
            &request,
            &[__dispatch_backend::<RecordingBackend>],
            &[ExtensionType::Backend],
        );

        assert_eq!(
            response.error.expect("method parameters should fail").code,
            protocol::error_codes::INVALID_PARAMS
        );
    }
}

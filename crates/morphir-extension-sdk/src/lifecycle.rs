//! Request ordering shared by generated guests and native protocol endpoints.

use std::sync::Mutex;

use crate::protocol::{ExtensionRequest, ExtensionResponse, RpcError, error_codes, methods};

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    #[default]
    BeforeInitialize,
    Initialized,
    Shutdown,
}

#[derive(Default)]
struct GuestState {
    lifecycle: Lifecycle,
    initialized: Option<crate::protocol::InitializeResult>,
    model: Option<crate::model::NegotiatedModelTransport>,
}

/// Session state used by the generated guest handler and native protocol adapter.
#[derive(Default)]
pub struct ProtocolSession {
    state: Mutex<GuestState>,
}

impl ProtocolSession {
    /// Create a session that has not yet initialized.
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(GuestState {
                lifecycle: Lifecycle::BeforeInitialize,
                initialized: None,
                model: None,
            }),
        }
    }

    /// Enforce request ordering and update state only after a successful response.
    ///
    /// The lock covers dispatch so initialization and shutdown cannot race work.
    /// The dispatch closure must not reenter this session. Exit closes the session
    /// without invoking the closure; the transport owns process termination.
    ///
    /// ```
    /// use morphir_extension_sdk::{__ProtocolSession, __dispatch_request, ExtensionType};
    /// use morphir_extension_sdk::native::doc_fixtures::DocFrontend;
    /// use morphir_extension_sdk::protocol::{ExtensionRequest, error_codes, methods};
    /// use serde_json::json;
    ///
    /// let session = __ProtocolSession::new();
    /// let handle = |method, params| {
    ///     let request = ExtensionRequest::new(method, params, 1).unwrap();
    ///     session.dispatch(&request, || {
    ///         __dispatch_request::<DocFrontend>(&request, &[], &[ExtensionType::Frontend])
    ///     })
    /// };
    /// assert_eq!(handle(methods::INFO, json!({})).error.unwrap().code,
    ///            error_codes::NOT_INITIALIZED);
    /// assert!(handle(methods::INITIALIZE, json!({
    ///     "protocolVersions": ["0.1"], "host": {"name": "example", "version": "1"}
    /// })).error.is_none());
    /// assert!(handle(methods::INFO, json!({})).error.is_none());
    /// assert!(handle(methods::SHUTDOWN, json!({})).error.is_none());
    /// assert_eq!(handle(methods::INFO, json!({})).error.unwrap().code,
    ///            error_codes::NOT_INITIALIZED);
    /// assert!(handle(methods::EXIT, json!({})).error.is_none());
    /// ```
    pub fn dispatch(
        &self,
        request: &ExtensionRequest,
        dispatch: impl FnOnce() -> ExtensionResponse,
    ) -> ExtensionResponse {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => {
                return ExtensionResponse::error(
                    request.id,
                    RpcError::internal_error("Extension session lock poisoned"),
                );
            }
        };
        let lifecycle = state.lifecycle;
        let method = request.method.as_str();
        let allowed = match lifecycle {
            Lifecycle::BeforeInitialize => matches!(
                method,
                methods::INITIALIZE | methods::DESCRIBE | methods::PING | methods::EXIT
            ),
            Lifecycle::Initialized => method != methods::INITIALIZE,
            Lifecycle::Shutdown => method == methods::EXIT,
        };
        if !allowed && lifecycle == Lifecycle::Initialized {
            return ExtensionResponse::error(
                request.id,
                RpcError::invalid_request("Extension is already initialized"),
            );
        }
        if !allowed {
            return ExtensionResponse::error(
                request.id,
                RpcError {
                    code: error_codes::NOT_INITIALIZED,
                    message: "Request is not allowed before initialization or after shutdown"
                        .into(),
                    data: None,
                },
            );
        }
        if method == methods::EXIT {
            state.lifecycle = Lifecycle::Shutdown;
            return ExtensionResponse::success(request.id, serde_json::json!({}))
                .expect("empty object is serializable");
        }
        if method == methods::CAPABILITIES
            && let Some(initialized) = &state.initialized
        {
            return ExtensionResponse::success(request.id, &initialized.capabilities)
                .expect("capabilities serialize");
        }
        if let Some(initialized) = &state.initialized {
            let allowed = match method {
                methods::COMPILE => initialized
                    .capabilities
                    .frontend
                    .as_ref()
                    .is_some_and(|capability| capability.compile),
                methods::GENERATE => initialized
                    .capabilities
                    .backend
                    .as_ref()
                    .is_some_and(|capability| capability.generate),
                _ => true,
            };
            if !allowed {
                return ExtensionResponse::error(
                    request.id,
                    RpcError {
                        code: error_codes::CAPABILITY_UNAVAILABLE,
                        message: "Operation was not advertised".into(),
                        data: None,
                    },
                );
            }
        }
        let is_model = matches!(method, methods::COMPILE_MODEL | methods::GENERATE_MODEL);
        if is_model {
            let Some(model) = &state.model else {
                return ExtensionResponse::error(
                    request.id,
                    RpcError {
                        code: error_codes::CAPABILITY_UNAVAILABLE,
                        message: "Model method was not negotiated".into(),
                        data: None,
                    },
                );
            };
            let operation = if method == methods::COMPILE_MODEL {
                crate::model::ModelOperation::CompileModel
            } else {
                crate::model::ModelOperation::GenerateModel
            };
            if !model
                .support()
                .tuples
                .iter()
                .any(|tuple| tuple.operation == operation)
            {
                return ExtensionResponse::error(
                    request.id,
                    RpcError {
                        code: error_codes::CAPABILITY_UNAVAILABLE,
                        message: "Model method was not negotiated".into(),
                        data: None,
                    },
                );
            }
            let initialized = state
                .initialized
                .as_ref()
                .expect("model transport is initialized");
            if let Err(error) = crate::model::validate_model_invocation(
                model,
                &initialized.capabilities,
                method,
                &request.params,
            ) {
                return ExtensionResponse::error(
                    request.id,
                    RpcError::invalid_params(error.to_string()),
                );
            }
        }
        let response = dispatch();
        if response.jsonrpc != crate::protocol::JSONRPC_VERSION || response.id != request.id {
            return ExtensionResponse::error(
                request.id,
                RpcError::internal_error("Invalid dispatch response envelope"),
            );
        }
        if response.error.is_none() {
            if is_model {
                if let Some(result) = &response.result {
                    if let Err(error) = crate::model::validate_model_result(
                        state.model.as_ref().expect("model session"),
                        method,
                        &request.params,
                        result,
                    ) {
                        return ExtensionResponse::error(
                            request.id,
                            RpcError::internal_error(error.to_string()),
                        );
                    }
                } else {
                    return ExtensionResponse::error(
                        request.id,
                        RpcError::internal_error("Missing model result"),
                    );
                }
            }
            match method {
                methods::INITIALIZE => {
                    let admitted = (|| {
                        let params: crate::protocol::InitializeParams =
                            serde_json::from_value(request.params.clone())
                                .map_err(|_| "Invalid initialization offer")?;
                        let initialized: crate::protocol::InitializeResult =
                            serde_json::from_value(
                                response
                                    .result
                                    .clone()
                                    .ok_or("Missing initialization result")?,
                            )
                            .map_err(|_| "Invalid initialization result")?;
                        if !params
                            .protocol_versions
                            .contains(&initialized.protocol_version)
                        {
                            return Err("Unoffered protocol version");
                        }
                        let model = crate::model::NegotiatedModelTransport::from_selection(
                            &initialized.protocol_version,
                            params.model_transport.as_ref(),
                            initialized.capabilities.model_transport.as_ref(),
                        )
                        .map_err(|_| "Invalid selected model transport")?;
                        Ok((initialized, model))
                    })();
                    match admitted {
                        Ok((initialized, model)) => {
                            state.initialized = Some(initialized);
                            state.model = model;
                            state.lifecycle = Lifecycle::Initialized;
                        }
                        Err(error) => {
                            return ExtensionResponse::error(
                                request.id,
                                RpcError::invalid_params(error),
                            );
                        }
                    }
                }
                methods::SHUTDOWN => state.lifecycle = Lifecycle::Shutdown,
                _ => {}
            }
        }
        response
    }
}

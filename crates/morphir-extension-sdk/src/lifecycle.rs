//! Request ordering shared by generated guests and native protocol endpoints.

use std::sync::Mutex;

use crate::protocol::{ExtensionRequest, ExtensionResponse, RpcError, error_codes, methods};

#[derive(Default, PartialEq, Eq)]
enum Lifecycle {
    #[default]
    BeforeInitialize,
    Initialized,
    Shutdown,
}

/// Session state used by the generated guest handler and native protocol adapter.
#[derive(Default)]
pub struct ProtocolSession {
    lifecycle: Mutex<Lifecycle>,
}

impl ProtocolSession {
    /// Create a session that has not yet initialized.
    pub const fn new() -> Self {
        Self {
            lifecycle: Mutex::new(Lifecycle::BeforeInitialize),
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
        let mut lifecycle = match self.lifecycle.lock() {
            Ok(lifecycle) => lifecycle,
            Err(_) => {
                return ExtensionResponse::error(
                    request.id,
                    RpcError::internal_error("Extension session lock poisoned"),
                );
            }
        };
        let method = request.method.as_str();
        let allowed = match *lifecycle {
            Lifecycle::BeforeInitialize => matches!(
                method,
                methods::INITIALIZE | methods::DESCRIBE | methods::PING | methods::EXIT
            ),
            Lifecycle::Initialized => method != methods::INITIALIZE,
            Lifecycle::Shutdown => method == methods::EXIT,
        };
        if !allowed && *lifecycle == Lifecycle::Initialized {
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
            *lifecycle = Lifecycle::Shutdown;
            return ExtensionResponse::success(request.id, serde_json::json!({}))
                .expect("empty object is serializable");
        }
        let response = dispatch();
        if response.error.is_none() {
            match method {
                methods::INITIALIZE => *lifecycle = Lifecycle::Initialized,
                methods::SHUTDOWN => *lifecycle = Lifecycle::Shutdown,
                _ => {}
            }
        }
        response
    }
}

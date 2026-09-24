//! Typed MEP operations over a negotiated connection.

use crate::connection::{CallError, GuestConnection};
use crate::{HostConfig, HostError, Negotiated};
use morphir_extension_sdk::protocol::methods;
use morphir_extension_sdk::{CompileRequest, CompileResult, GenerateRequest, GenerateResult};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Combine a call failure with a failure to shut down in order.
///
/// The combined error keeps the channel state of the shutdown failure, since
/// that is the failure that describes the transport now.
fn also_failed_to_shut_down(error: HostError, close: HostError) -> HostError {
    let message = format!("{error}; orderly shutdown also failed: {close}");
    match close {
        HostError::Channel { state, .. } => HostError::Channel { message, state },
        _ => HostError::Invalid(message),
    }
}

/// One live guest after the handshake.
///
/// The client owns the session and closes it. A client may hold many sessions.
///
/// # Example
///
/// ```
/// # tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
/// use morphir_extension_sdk::protocol::{ExtensionResponse, PeerInfo, PeerKind};
/// use morphir_extension_sdk::{CompileRequest, CompileResult};
/// use morphir_host::testing::{MemoryChannel, frontend_initialize_result};
/// use morphir_host::{BasicChecks, HostConfig, JsonRpcConnection, Session};
///
/// fn ok(id: u64, value: impl serde::Serialize) -> ExtensionResponse {
///     ExtensionResponse::success(id, value).unwrap()
/// }
///
/// fn compiled() -> CompileResult {
///     CompileResult {
///         success: true,
///         ir_version: Some("4.0.0".into()),
///         ir: Some(serde_json::json!({})),
///         diagnostics: Vec::new(),
///         modules: vec!["Example".into()],
///         module_results: Vec::new(),
///         context_digest: None,
///     }
/// }
///
/// // The channel answers initialize (id 1), compile (id 2) and shutdown (id 3).
/// let channel = MemoryChannel::new()
///     .respond(ok(1, frontend_initialize_result("example-guest")))
///     .respond(ok(2, compiled()))
///     .respond(ok(3, serde_json::json!({})));
/// let connection = JsonRpcConnection::new(channel, BasicChecks::new("example-guest"));
///
/// let config = HostConfig::new(PeerInfo {
///     kind: PeerKind::Unspecified,
///     name: "example-host".into(),
///     version: "1.0.0".into(),
/// });
/// let mut session = Session::open(connection, &config).await.unwrap();
///
/// let result = session.compile(CompileRequest::default()).await.unwrap();
/// assert!(result.success);
///
/// session.close().await.unwrap();
/// # });
/// ```
pub struct Session {
    connection: Box<dyn GuestConnection>,
    negotiated: Negotiated,
}

impl Session {
    /// Run the handshake on `connection` as the host that `config` describes.
    pub async fn open<G: GuestConnection + 'static>(
        mut connection: G,
        config: &HostConfig,
    ) -> Result<Self, HostError> {
        let negotiated = connection.open(config.initialize_params()).await?;
        Ok(Self {
            connection: Box::new(connection),
            negotiated,
        })
    }

    /// What the guest agreed to in the handshake.
    pub fn negotiated(&self) -> &Negotiated {
        &self.negotiated
    }

    /// Invoke one non-lifecycle method with typed parameters and result.
    ///
    /// A result that does not decode into `R` ends the session: the guest
    /// answered with a valid envelope, so the session closes it in order
    /// before reporting the decode failure.
    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: P,
    ) -> Result<R, CallError> {
        let params =
            serde_json::to_value(params).map_err(|error| CallError::Rejected(error.into()))?;
        let value = self.connection.call(method, params).await?;
        match serde_json::from_value(value) {
            Ok(result) => Ok(result),
            Err(error) => {
                let error = HostError::from(error);
                match self.connection.close().await {
                    Ok(()) => Err(CallError::Failed(error)),
                    Err(close) => Err(CallError::Failed(also_failed_to_shut_down(error, close))),
                }
            }
        }
    }

    /// Compile sources with a frontend guest.
    pub async fn compile(&mut self, request: CompileRequest) -> Result<CompileResult, CallError> {
        self.call(methods::COMPILE, request).await
    }

    /// Generate code with a backend guest.
    pub async fn generate(
        &mut self,
        request: GenerateRequest,
    ) -> Result<GenerateResult, CallError> {
        self.call(methods::GENERATE, request).await
    }

    /// Complete MEP shutdown and release the guest.
    pub async fn close(mut self) -> Result<(), HostError> {
        self.connection.close().await
    }
}

/// Open a session, make one call, and close the session.
///
/// A rejected call still shuts the guest down in order. If that shutdown also
/// fails, the error names both failures.
pub async fn call_once<G, P, R>(
    connection: G,
    config: &HostConfig,
    method: &str,
    params: P,
) -> Result<R, HostError>
where
    G: GuestConnection + 'static,
    P: Serialize,
    R: DeserializeOwned,
{
    let mut session = Session::open(connection, config).await?;
    match session.call(method, params).await {
        Ok(result) => {
            session.close().await?;
            Ok(result)
        }
        Err(CallError::Rejected(error)) => match session.close().await {
            Ok(()) => Err(error),
            Err(close) => Err(also_failed_to_shut_down(error, close)),
        },
        Err(CallError::Failed(error)) => Err(error),
    }
}

/// Compile once with a fresh guest.
pub async fn compile_once<G: GuestConnection + 'static>(
    connection: G,
    config: &HostConfig,
    request: CompileRequest,
) -> Result<CompileResult, HostError> {
    call_once(connection, config, methods::COMPILE, request).await
}

/// Generate once with a fresh guest.
pub async fn generate_once<G: GuestConnection + 'static>(
    connection: G,
    config: &HostConfig,
    request: GenerateRequest,
) -> Result<GenerateResult, HostError> {
    call_once(connection, config, methods::GENERATE, request).await
}

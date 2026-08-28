//! JSON-RPC HTTP transport for independently hosted extension daemons.

use crate::extensions::protocol::{
    ExtensionRequest, ExtensionResponse, MAX_MEP_PAYLOAD_BYTES, RpcError, methods,
};
use crate::extensions::session::{
    ExpectedExtension, Loaded, MepTransport, Session, TransportError, TransportState,
};
use crate::{DaemonError, Result};
use async_trait::async_trait;
use jsonrpsee::core::{ClientError, client::ClientT};
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use std::time::Duration;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Connection settings for an extension daemon reached over JSON-RPC HTTP.
#[derive(Debug, Clone)]
pub struct DaemonConnection {
    extension_id: String,
    endpoint: String,
    request_timeout: Duration,
}

impl DaemonConnection {
    /// Define an expected extension identity and HTTP endpoint.
    pub fn new(extension_id: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self {
            extension_id: extension_id.into(),
            endpoint: endpoint.into(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Set the timeout applied to each HTTP request.
    pub fn request_timeout(mut self, request_timeout: Duration) -> Self {
        self.request_timeout = request_timeout;
        self
    }
}

/// Factory for HTTP daemon typestate sessions.
///
/// ```no_run
/// # async fn example() -> morphir_daemon::Result<()> {
/// use morphir_daemon::{DaemonError, extensions::{ConnectedDaemonSession, DaemonConnection}};
/// use morphir_extension_sdk::protocol::{InitializeParams, PeerInfo};
///
/// let loaded = ConnectedDaemonSession::connect(DaemonConnection::new(
///     "example-backend",
///     "http://127.0.0.1:9741",
/// ))?;
/// let ready = loaded.initialize(InitializeParams {
///     protocol_versions: vec!["0.1".into()],
///     host: PeerInfo {
///         name: "example-host".into(),
///         version: "1.0.0".into(),
///     },
/// }).await.map_err(|failure| DaemonError::Extension(failure.error().to_string()))?;
/// let _stopped = ready.shutdown().await
///     .map_err(|failure| DaemonError::Extension(failure.error().to_string()))?;
/// # Ok(())
/// # }
/// ```
pub struct ConnectedDaemonSession;

impl ConnectedDaemonSession {
    /// Configure an HTTP transport in the loaded state.
    ///
    /// The first request performs the network connection. This constructor
    /// validates the endpoint without changing the remote daemon.
    pub fn connect(
        connection: DaemonConnection,
    ) -> Result<Session<ConnectedDaemonTransport, Loaded>> {
        if connection.extension_id.trim().is_empty() {
            return Err(DaemonError::Extension(
                "Extension daemon identity cannot be empty".to_string(),
            ));
        }
        let client = HttpClientBuilder::default()
            .request_timeout(connection.request_timeout)
            .max_request_size(MAX_MEP_PAYLOAD_BYTES)
            .max_response_size(MAX_MEP_PAYLOAD_BYTES)
            .build(&connection.endpoint)
            .map_err(|error| {
                DaemonError::Extension(format!(
                    "Invalid extension daemon endpoint '{}': {error}",
                    connection.endpoint
                ))
            })?;

        Ok(Session::loaded(ConnectedDaemonTransport {
            expected_extension_id: connection.extension_id,
            client,
            shutdown_acknowledged: false,
        }))
    }
}

/// JSON-RPC HTTP implementation of the object-safe MEP transport.
pub struct ConnectedDaemonTransport {
    expected_extension_id: String,
    client: HttpClient,
    shutdown_acknowledged: bool,
}

#[async_trait]
impl MepTransport for ConnectedDaemonTransport {
    fn expected_extension(&self) -> ExpectedExtension {
        ExpectedExtension::identified(self.expected_extension_id.clone())
    }

    async fn exchange(
        &mut self,
        request: ExtensionRequest,
    ) -> std::result::Result<ExtensionResponse, TransportError> {
        let id = request.id;
        let method = request.method;
        let Some(params) = request.params.as_object().cloned() else {
            return Ok(ExtensionResponse::error(
                id,
                RpcError::invalid_params("Extension request parameters must be an object"),
            ));
        };

        match self
            .client
            .request::<serde_json::Value, _>(&method, params)
            .await
        {
            Ok(result) => {
                if method == methods::SHUTDOWN {
                    self.shutdown_acknowledged = true;
                }
                ExtensionResponse::success(id, result).map_err(|error| {
                    TransportError::new(error.into(), TransportState::Indeterminate)
                })
            }
            Err(ClientError::Call(error)) => Ok(ExtensionResponse::error(
                id,
                RpcError {
                    code: error.code(),
                    message: error.message().to_string(),
                    data: None,
                },
            )),
            Err(error) => Err(TransportError::new(
                DaemonError::Extension(format!(
                    "HTTP extension request '{method}' failed: {error}"
                )),
                TransportState::Indeterminate,
            )),
        }
    }

    async fn terminate(&mut self) -> std::result::Result<TransportState, TransportError> {
        Ok(if self.shutdown_acknowledged {
            TransportState::Stopped
        } else {
            TransportState::Indeterminate
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_an_empty_expected_identity() {
        let error =
            ConnectedDaemonSession::connect(DaemonConnection::new("  ", "http://127.0.0.1:9741"))
                .err()
                .expect("an empty identity should fail");
        assert!(error.to_string().contains("identity cannot be empty"));
    }

    #[test]
    fn rejects_an_invalid_endpoint() {
        let error = ConnectedDaemonSession::connect(DaemonConnection::new("example", "not a URL"))
            .err()
            .expect("an invalid endpoint should fail");
        assert!(
            error
                .to_string()
                .contains("Invalid extension daemon endpoint")
        );
    }
}

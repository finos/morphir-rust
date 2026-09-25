//! Reaching an extension over JSON-RPC HTTP.
//!
//! [`HttpChannel`] is a `morphir_host::Channel` for an extension that runs
//! as its own server, reached at a URL. It needs the `http` feature.

use crate::process::MAX_MEP_PAYLOAD_BYTES;
use crate::results::CheckedConnection;
use async_trait::async_trait;
use jsonrpsee::core::{ClientError, client::ClientT};
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use morphir_extension_sdk::protocol::{ExtensionResponse, RpcError, methods};
use morphir_host::{
    Channel, ChannelCause, ChannelError, ChannelState, ExpectedChecks, ExpectedExtension,
    HostError, JsonRpcConnection, Outgoing,
};
use serde_json::Value;
use std::time::Duration;

/// The time an [`HttpEndpoint`] made with [`HttpEndpoint::new`] waits for
/// each request.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Where an extension listens for JSON-RPC over HTTP, and which extension
/// the host expects to find there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpEndpoint {
    /// The extension id the handshake must report.
    pub id: String,
    /// The URL of the extension's JSON-RPC endpoint.
    pub url: String,
    /// How long each request may take before the channel fails.
    pub request_timeout: Duration,
}

impl HttpEndpoint {
    /// An endpoint with the [`DEFAULT_REQUEST_TIMEOUT`].
    pub fn new(id: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            url: url.into(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }
}

/// A channel to an extension reached over JSON-RPC HTTP.
///
/// Each request is one HTTP POST. The channel keeps the reply until the host
/// asks for it, with the host's request id. Requests and replies may each be
/// up to [`MAX_MEP_PAYLOAD_BYTES`].
///
/// - A notification is not sent. HTTP has no `morphir.exit`: the extension
///   decides on its own when to stop after `morphir.shutdown`.
/// - A request whose parameters are not a JSON object is not sent. Its reply
///   is an `invalid_params` error.
/// - A JSON-RPC error reply is an error response. The session stays ready.
/// - Any other failure, such as a refused connection or a timeout, is a
///   channel failure in the `Indeterminate` state: the host cannot prove
///   whether the extension accepted the request.
/// - `close` reports `Stopped` only when the last reply was a successful
///   answer to `morphir.shutdown`. `abort` sends nothing.
///
/// # Example
///
/// ```no_run
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// use morphir_extension_sdk::protocol::{PeerInfo, PeerKind};
/// use morphir_host::{HostConfig, Session};
/// use morphir_host_native::http::{HttpChannel, HttpEndpoint};
///
/// let endpoint = HttpEndpoint::new("example-backend", "http://127.0.0.1:9741");
/// let channel = HttpChannel::connect(endpoint)?;
///
/// let config = HostConfig::new(PeerInfo {
///     kind: PeerKind::Unspecified,
///     name: "example-host".into(),
///     version: "1.0.0".into(),
/// });
/// let session = Session::open(channel.connection(), &config).await?;
/// session.close().await?;
/// # Ok(())
/// # }
/// ```
pub struct HttpChannel {
    id: String,
    client: HttpClient,
    reply: Option<ExtensionResponse>,
    shutdown_acknowledged: bool,
}

impl HttpChannel {
    /// Check `endpoint` and build its HTTP client.
    ///
    /// The id must not be blank and the URL must parse; otherwise the error
    /// is `HostError::Rejected`. Nothing is sent: the first request makes
    /// the first connection.
    pub fn connect(endpoint: HttpEndpoint) -> Result<Self, HostError> {
        if endpoint.id.trim().is_empty() {
            return Err(HostError::Rejected(
                "HTTP extension identity cannot be empty".into(),
            ));
        }
        let client = HttpClientBuilder::default()
            .request_timeout(endpoint.request_timeout)
            .max_request_size(MAX_MEP_PAYLOAD_BYTES)
            .max_response_size(MAX_MEP_PAYLOAD_BYTES)
            .build(&endpoint.url)
            .map_err(|error| {
                HostError::Rejected(format!(
                    "Invalid HTTP extension endpoint '{}': {error}",
                    endpoint.url
                ))
            })?;
        Ok(Self {
            id: endpoint.id,
            client,
            reply: None,
            shutdown_acknowledged: false,
        })
    }

    /// A checked connection to this channel's extension.
    ///
    /// The handshake must report the id of the endpoint, and every call
    /// result is checked as for any other guest.
    pub fn connection(
        self,
    ) -> CheckedConnection<JsonRpcConnection<Box<dyn Channel>, ExpectedChecks>> {
        let expected = ExpectedExtension::identified(self.id.clone());
        let channel: Box<dyn Channel> = Box::new(self);
        CheckedConnection::new(JsonRpcConnection::new(
            channel,
            ExpectedChecks::new(expected),
        ))
    }
}

fn indeterminate(message: String, cause: ChannelCause) -> ChannelError {
    ChannelError {
        message,
        state: ChannelState::Indeterminate,
        cause,
    }
}

#[async_trait]
impl Channel for HttpChannel {
    async fn send(&mut self, message: Outgoing) -> Result<(), ChannelError> {
        let request = match message {
            Outgoing::Request(request) => request,
            Outgoing::Notification(_) => return Ok(()),
        };
        self.reply = None;
        self.shutdown_acknowledged = false;
        let id = request.id;
        let method = request.method;
        let Value::Object(params) = request.params else {
            self.reply = Some(ExtensionResponse::error(
                id,
                RpcError::invalid_params("Extension request parameters must be an object"),
            ));
            return Ok(());
        };

        let reply = match self.client.request::<Value, _>(&method, params).await {
            Ok(result) => {
                let reply = ExtensionResponse::success(id, result).map_err(|error| {
                    indeterminate(HostError::from(error).to_string(), ChannelCause::Json)
                })?;
                self.shutdown_acknowledged = method == methods::SHUTDOWN;
                reply
            }
            Err(ClientError::Call(error)) => ExtensionResponse::error(
                id,
                RpcError {
                    code: error.code(),
                    message: error.message().to_owned(),
                    data: None,
                },
            ),
            Err(error) => {
                return Err(indeterminate(
                    format!("HTTP extension request '{method}' failed: {error}"),
                    ChannelCause::Transport,
                ));
            }
        };
        self.reply = Some(reply);
        Ok(())
    }

    async fn receive(&mut self) -> Result<ExtensionResponse, ChannelError> {
        self.reply.take().ok_or_else(|| {
            indeterminate(
                "HTTP extension channel has no response ready".into(),
                ChannelCause::Transport,
            )
        })
    }

    async fn close(&mut self) -> Result<ChannelState, ChannelError> {
        Ok(if self.shutdown_acknowledged {
            ChannelState::Stopped
        } else {
            ChannelState::Indeterminate
        })
    }

    /// Send nothing. The extension runs on its own, so the host cannot stop
    /// it, and cannot prove it stopped.
    async fn abort(&mut self) -> Result<ChannelState, ChannelError> {
        Ok(ChannelState::Indeterminate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonrpsee::server::{RpcModule, ServerBuilder, ServerHandle};
    use jsonrpsee::types::ErrorObjectOwned;
    use morphir_extension_sdk::CompileRequest;
    use morphir_extension_sdk::protocol::{ExtensionRequest, PeerInfo};
    use morphir_host::testing::frontend_initialize_result;
    use morphir_host::{CallError, HostConfig, Session};

    const REFUSED: i32 = -32001;

    fn config() -> HostConfig {
        HostConfig::new(PeerInfo {
            kind: Default::default(),
            name: "http-channel-test".into(),
            version: "0.1.0".into(),
        })
    }

    /// A frontend on a local port that answers every compile with a
    /// JSON-RPC error.
    async fn a_frontend_that_refuses_compile() -> (String, ServerHandle) {
        let mut module = RpcModule::new(());
        module
            .register_method(methods::INITIALIZE, |_, _, _| {
                serde_json::to_value(frontend_initialize_result("http-guest"))
                    .expect("the initialize result should encode")
            })
            .unwrap();
        module
            .register_method(methods::COMPILE, |_, _, _| {
                Err::<Value, _>(ErrorObjectOwned::owned(
                    REFUSED,
                    "compile refused",
                    None::<()>,
                ))
            })
            .unwrap();
        module
            .register_method(methods::SHUTDOWN, |_, _, _| serde_json::json!({}))
            .unwrap();
        let server = ServerBuilder::default()
            .build("127.0.0.1:0")
            .await
            .expect("a loopback port should be available");
        let url = format!("http://{}", server.local_addr().unwrap());
        (url, server.start(module))
    }

    #[tokio::test]
    async fn a_json_rpc_error_reply_is_an_error_response() {
        let (url, server) = a_frontend_that_refuses_compile().await;
        let channel = HttpChannel::connect(HttpEndpoint::new("http-guest", url)).unwrap();
        let mut session = Session::open(channel.connection(), &config())
            .await
            .expect("the handshake should succeed");

        let error = session
            .compile(CompileRequest::default())
            .await
            .expect_err("the guest refuses every compile");

        match error {
            CallError::Rejected(HostError::Rpc(error)) => {
                assert_eq!(error.code, REFUSED);
                assert_eq!(error.message, "compile refused");
            }
            other => panic!("expected the guest's RPC error, got {other:?}"),
        }
        session
            .close()
            .await
            .expect("the session should still close in order");
        server.stop().unwrap();
    }

    #[tokio::test]
    async fn a_closed_port_is_an_indeterminate_transport_failure() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let mut channel = HttpChannel::connect(HttpEndpoint::new("http-guest", url)).unwrap();
        let request = ExtensionRequest::new(methods::INITIALIZE, serde_json::json!({}), 1).unwrap();

        let error = channel
            .send(Outgoing::Request(request))
            .await
            .expect_err("nothing listens on the port");

        assert_eq!(error.state, ChannelState::Indeterminate);
        assert_eq!(error.cause, ChannelCause::Transport);
        assert!(error.message.contains("HTTP extension request"));
    }

    #[tokio::test]
    async fn rejects_an_empty_expected_identity() {
        let error = HttpChannel::connect(HttpEndpoint::new("  ", "http://127.0.0.1:9741"))
            .err()
            .expect("an empty identity should fail");
        assert!(error.to_string().contains("identity cannot be empty"));
    }

    #[tokio::test]
    async fn rejects_an_invalid_endpoint() {
        let error = HttpChannel::connect(HttpEndpoint::new("example", "not a URL"))
            .err()
            .expect("an invalid endpoint should fail");
        assert!(
            error
                .to_string()
                .contains("Invalid HTTP extension endpoint")
        );
    }
}

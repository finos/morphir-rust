//! JSON-RPC HTTP transport for independently hosted extension daemons.

use crate::extensions::protocol::{
    InitializeParams, InitializeResult, MAX_MEP_PAYLOAD_BYTES, error_codes, methods,
};
use crate::extensions::session::{ExtensionSession, ExtensionSessionState};
use crate::{DaemonError, Result};
use async_trait::async_trait;
use jsonrpsee::core::{ClientError, client::ClientT};
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use morphir_extension_sdk::ExtensionType;
use serde::Serialize;
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

enum ConnectedSessionData {
    Starting,
    Ready(Box<InitializeResult>),
    Stopped,
}

/// A MEP session connected to an independently hosted extension daemon.
///
/// ```no_run
/// # async fn example() -> morphir_daemon::Result<()> {
/// use morphir_daemon::extensions::{
///     ConnectedDaemonSession, DaemonConnection, ExtensionSession,
/// };
/// use morphir_extension_sdk::protocol::{InitializeParams, PeerInfo};
///
/// let connection = DaemonConnection::new(
///     "example-backend",
///     "http://127.0.0.1:9741",
/// );
/// let mut session = ConnectedDaemonSession::connect(connection)?;
/// session.initialize(InitializeParams {
///     protocol_versions: vec!["0.1".into()],
///     host: PeerInfo {
///         name: "example-host".into(),
///         version: "1.0.0".into(),
///     },
/// }).await?;
/// session.shutdown().await?;
/// # Ok(())
/// # }
/// ```
pub struct ConnectedDaemonSession {
    expected_extension_id: String,
    client: HttpClient,
    state: ConnectedSessionData,
}

impl ConnectedDaemonSession {
    /// Configure a JSON-RPC HTTP client for an extension daemon.
    ///
    /// The first request performs the network connection. This constructor
    /// validates the endpoint and prepares the client without changing the
    /// remote daemon.
    pub fn connect(connection: DaemonConnection) -> Result<Self> {
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

        Ok(Self {
            expected_extension_id: connection.extension_id,
            client,
            state: ConnectedSessionData::Starting,
        })
    }

    fn ready_session(&self) -> Result<&InitializeResult> {
        match &self.state {
            ConnectedSessionData::Ready(initialized) => Ok(initialized),
            ConnectedSessionData::Starting | ConnectedSessionData::Stopped => Err(
                DaemonError::Extension("Extension session is not ready".to_string()),
            ),
        }
    }

    async fn call<P>(&mut self, method: &str, params: P) -> Result<serde_json::Value>
    where
        P: Serialize,
    {
        let params = serde_json::to_value(params)?;
        let params = params.as_object().cloned().ok_or_else(|| {
            DaemonError::Extension("Extension request parameters must be an object".to_string())
        })?;
        match self.client.request(method, params).await {
            Ok(result) => Ok(result),
            Err(error) => {
                if !matches!(error, ClientError::Call(_)) {
                    self.state = ConnectedSessionData::Stopped;
                }
                Err(DaemonError::Extension(format!(
                    "HTTP extension request '{method}' failed: {error}"
                )))
            }
        }
    }
}

#[async_trait]
impl ExtensionSession for ConnectedDaemonSession {
    fn state(&self) -> ExtensionSessionState {
        match self.state {
            ConnectedSessionData::Starting => ExtensionSessionState::Starting,
            ConnectedSessionData::Ready(_) => ExtensionSessionState::Ready,
            ConnectedSessionData::Stopped => ExtensionSessionState::Stopped,
        }
    }

    async fn initialize(&mut self, params: InitializeParams) -> Result<InitializeResult> {
        if !matches!(self.state, ConnectedSessionData::Starting) {
            return Err(DaemonError::Extension(
                "Extension session can only initialize once".to_string(),
            ));
        }

        let offered_versions = params.protocol_versions.clone();
        let initialized = serde_json::from_value::<InitializeResult>(
            self.call(methods::INITIALIZE, params).await?,
        )?;
        if !offered_versions.contains(&initialized.protocol_version) {
            self.state = ConnectedSessionData::Stopped;
            return Err(DaemonError::Extension(format!(
                "Extension selected protocol version '{}' that the host did not offer",
                initialized.protocol_version
            )));
        }
        if initialized.extension.id != self.expected_extension_id {
            self.state = ConnectedSessionData::Stopped;
            return Err(DaemonError::Extension(format!(
                "Extension identity changed during initialization: expected '{}', initialized '{}'",
                self.expected_extension_id, initialized.extension.id
            )));
        }

        self.state = ConnectedSessionData::Ready(Box::new(initialized.clone()));
        Ok(initialized)
    }

    async fn invoke(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let initialized = self.ready_session()?;
        if matches!(method, methods::INITIALIZE | methods::SHUTDOWN) {
            return Err(DaemonError::Extension(format!(
                "Protocol lifecycle method '{method}' must use its dedicated session operation"
            )));
        }
        if let Some(required) = required_capability(method)
            && !initialized.extension.types.contains(&required)
        {
            return Err(DaemonError::Extension(format!(
                "RPC error {}: Extension '{}' does not support capability '{}'",
                error_codes::CAPABILITY_UNAVAILABLE,
                initialized.extension.id,
                method
            )));
        }

        self.call(method, params).await
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.ready_session()?;
        self.call(methods::SHUTDOWN, serde_json::json!({})).await?;
        self.state = ConnectedSessionData::Stopped;
        Ok(())
    }
}

fn required_capability(method: &str) -> Option<ExtensionType> {
    match method {
        methods::COMPILE => Some(ExtensionType::Frontend),
        methods::GENERATE => Some(ExtensionType::Backend),
        methods::VALIDATE => Some(ExtensionType::Validator),
        methods::TRANSFORM => Some(ExtensionType::Transform),
        _ => None,
    }
}

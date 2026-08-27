//! Transport-independent Morphir Extension Protocol session lifecycle.

use crate::extensions::ExtensionContainer;
use crate::extensions::protocol::{InitializeParams, InitializeResult, error_codes, methods};
use crate::{DaemonError, Result};
use async_trait::async_trait;
use morphir_extension_sdk::ExtensionType;

/// Observable lifecycle state of an extension session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionSessionState {
    /// The extension has loaded but has not negotiated MEP.
    Starting,
    /// The extension has negotiated MEP and accepts operations.
    Ready,
    /// The extension has completed shutdown.
    Stopped,
}

enum SessionData {
    Starting,
    Ready(Box<InitializeResult>),
    Stopped,
}

/// Common session operations implemented by each extension transport adapter.
#[async_trait]
pub trait ExtensionSession {
    /// Return the current lifecycle state.
    fn state(&self) -> ExtensionSessionState;

    /// Negotiate a protocol version and session capabilities.
    async fn initialize(&mut self, params: InitializeParams) -> Result<InitializeResult>;

    /// Invoke one operation using JSON values from the shared protocol.
    async fn invoke(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value>;

    /// Stop accepting operations and complete the session.
    async fn shutdown(&mut self) -> Result<()>;
}

/// MEP session carried by an in-process Extism plugin.
///
/// ```no_run
/// # async fn example() -> morphir_daemon::Result<()> {
/// use morphir_daemon::ExtensionContainer;
/// use morphir_daemon::extensions::{
///     ExtensionSession, ExtismSession,
///     host_functions::MorphirHostFunctions,
/// };
/// use morphir_extension_sdk::protocol::{InitializeParams, PeerInfo};
///
/// let container = ExtensionContainer::new(
///     "example",
///     std::path::Path::new("example.wasm"),
///     MorphirHostFunctions::default(),
/// )?;
/// let mut session = ExtismSession::new(container);
/// session.initialize(InitializeParams {
///     protocol_versions: vec!["0.1".into()],
///     host: PeerInfo { name: "example-host".into(), version: "1.0.0".into() },
/// }).await?;
/// session.shutdown().await?;
/// # Ok(())
/// # }
/// ```
pub struct ExtismSession {
    container: ExtensionContainer,
    state: SessionData,
}

impl ExtismSession {
    /// Wrap a loaded Extism container in a new MEP session.
    pub fn new(container: ExtensionContainer) -> Self {
        Self {
            container,
            state: SessionData::Starting,
        }
    }

    fn ready_session(&self) -> Result<&InitializeResult> {
        match &self.state {
            SessionData::Ready(initialized) => Ok(initialized),
            SessionData::Starting | SessionData::Stopped => Err(DaemonError::Extension(
                "Extension session is not ready".to_string(),
            )),
        }
    }
}

#[async_trait]
impl ExtensionSession for ExtismSession {
    fn state(&self) -> ExtensionSessionState {
        match self.state {
            SessionData::Starting => ExtensionSessionState::Starting,
            SessionData::Ready(_) => ExtensionSessionState::Ready,
            SessionData::Stopped => ExtensionSessionState::Stopped,
        }
    }

    async fn initialize(&mut self, params: InitializeParams) -> Result<InitializeResult> {
        if !matches!(self.state, SessionData::Starting) {
            return Err(DaemonError::Extension(
                "Extension session can only initialize once".to_string(),
            ));
        }

        let offered_versions = params.protocol_versions.clone();
        let initialized: InitializeResult =
            self.container.call(methods::INITIALIZE, params).await?;
        if !offered_versions.contains(&initialized.protocol_version) {
            return Err(DaemonError::Extension(format!(
                "Extension selected protocol version '{}' that the host did not offer",
                initialized.protocol_version
            )));
        }
        if initialized.extension.id != self.container.info().id {
            return Err(DaemonError::Extension(format!(
                "Extension identity changed during initialization: loaded '{}', initialized '{}'",
                self.container.info().id,
                initialized.extension.id
            )));
        }

        self.state = SessionData::Ready(Box::new(initialized.clone()));
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
                "Protocol lifecycle method '{}' must use its dedicated session operation",
                method
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

        self.container.call(method, params).await
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.ready_session()?;
        let _: serde_json::Value = self
            .container
            .call(methods::SHUTDOWN, serde_json::json!({}))
            .await?;
        self.state = SessionData::Stopped;
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

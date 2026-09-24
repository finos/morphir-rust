//! One-shot process description with the optional-method session fallback.

use super::*;
use morphir_extension_sdk::claims::CapabilityClaimSet;
use morphir_host::{Channel, ChannelError, ChannelState, HostConfig, Outgoing};

/// How a process supplied its capability claim set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptionSource {
    /// The guest answered `morphir.extension.describe` directly.
    Describe,
    /// The host reconstructed the claims from a negotiated session.
    SessionFallback,
}

/// A process claim set and the operation that produced it.
#[derive(Debug, Clone)]
pub struct ProcessDescription {
    /// The guest's claims, or the subset reported by a fallback session.
    pub claims: CapabilityClaimSet,
    /// Distinguishes direct descriptions from session reconstructions.
    pub source: DescriptionSource,
}

impl From<morphir_host::Description> for ProcessDescription {
    fn from(description: morphir_host::Description) -> Self {
        Self {
            claims: description.claims,
            source: match description.source {
                morphir_host::DescriptionSource::Describe => DescriptionSource::Describe,
                morphir_host::DescriptionSource::SessionFallback => {
                    DescriptionSource::SessionFallback
                }
            },
        }
    }
}

impl SpawnedProcessTransport {
    /// Describe a fresh process, falling back only when describe is unavailable.
    ///
    /// Consumes the transport and completes `exit` before returning. The fallback
    /// sends initialize, initialized, capabilities, shutdown, and exit in order.
    /// Launch policy and request timeouts are the same as for ordinary sessions.
    ///
    /// ```no_run
    /// # async fn example() -> morphir_daemon::Result<()> {
    /// use morphir_daemon::extensions::{ProcessLaunch, SpawnedProcessTransport};
    /// use morphir_extension_sdk::protocol::{InitializeParams, PeerInfo};
    /// let transport = SpawnedProcessTransport::spawn(ProcessLaunch::new(
    ///     "example", "/extensions/example", "/workspace",
    /// )).await?;
    /// let description = transport.describe(InitializeParams {
    ///     protocol_versions: vec!["0.1".into()],
    ///     host: PeerInfo { kind: Default::default(), name: "host".into(), version: "1.0.0".into() },
    /// }).await?;
    /// assert_eq!(description.claims.extension.id, "example");
    /// # Ok(()) }
    /// ```
    pub async fn describe(mut self, params: InitializeParams) -> Result<ProcessDescription> {
        let config = HostConfig::with_versions(params.host, params.protocol_versions);
        let expected_id = self.expected_extension().id().to_owned();
        let mut failure = None;
        let channel = TransportChannel {
            transport: &mut self,
            request: None,
            failure: &mut failure,
        };
        match morphir_host::describe(channel, &config, &expected_id).await {
            Ok(description) => Ok(description.into()),
            Err(error) => {
                // A channel failure keeps the daemon error it came from, so its
                // variant and text are the same as before the probe moved.
                let error = failure.unwrap_or_else(|| error.into());
                Err(self.session.abort_with_error(error).await)
            }
        }
    }
}

/// The describe probe's view of a [`SpawnedProcessTransport`].
///
/// `send` keeps a request and `receive` exchanges it, so each request is
/// written and answered under one timeout, as in an ordinary session. `close`
/// waits for the exit that the probe already sent. Each failure stores its
/// daemon error in `failure`, because a `ChannelError` keeps only the text.
struct TransportChannel<'a> {
    transport: &'a mut SpawnedProcessTransport,
    request: Option<ExtensionRequest>,
    failure: &'a mut Option<DaemonError>,
}

impl TransportChannel<'_> {
    fn fail(&mut self, error: TransportError) -> ChannelError {
        let state = match error.state() {
            TransportState::Stopped => ChannelState::Stopped,
            TransportState::Indeterminate => ChannelState::Indeterminate,
        };
        let error = error.into_error();
        let failure = ChannelError {
            message: error.to_string(),
            state,
        };
        *self.failure = Some(error);
        failure
    }
}

#[async_trait]
impl Channel for TransportChannel<'_> {
    async fn send(&mut self, message: Outgoing) -> std::result::Result<(), ChannelError> {
        let notification = match message {
            Outgoing::Request(request) => {
                self.request = Some(request);
                return Ok(());
            }
            Outgoing::Notification(notification) => notification,
        };
        let child = &mut self.transport.session.child;
        let error = match child.write(&notification).await {
            Ok(()) => return Ok(()),
            Err(HostError::Channel { .. }) if notification.method == methods::EXIT => {
                DaemonError::Extension(format!(
                    "Extension exit notification timed out after {:?}",
                    child.request_timeout()
                ))
            }
            Err(HostError::Channel { .. }) if notification.method == methods::INITIALIZED => {
                DaemonError::Extension("Extension initialized notification timed out".into())
            }
            Err(error) => error.into(),
        };
        let error = self.transport.stop_after(error).await;
        Err(self.fail(error))
    }

    async fn receive(&mut self) -> std::result::Result<ExtensionResponse, ChannelError> {
        let Some(request) = self.request.take() else {
            let error = DaemonError::Extension("Extension channel has no request to answer".into());
            let error = self.transport.stop_after(error).await;
            return Err(self.fail(error));
        };
        match self.transport.exchange(request).await {
            Ok(response) => Ok(response),
            Err(error) => Err(self.fail(error)),
        }
    }

    async fn close(&mut self) -> std::result::Result<ChannelState, ChannelError> {
        match self.transport.finish_after_exit().await {
            Ok(_) => Ok(ChannelState::Stopped),
            Err(error) => Err(self.fail(error)),
        }
    }

    async fn abort(&mut self) -> std::result::Result<ChannelState, ChannelError> {
        match MepTransport::abort(self.transport).await {
            Ok(_) => Ok(ChannelState::Stopped),
            Err(error) => Err(ChannelError {
                message: error.into_error().to_string(),
                state: ChannelState::Indeterminate,
            }),
        }
    }
}

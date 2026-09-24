//! The JSON-RPC binding of MEP over any [`Channel`].

use crate::channel::{Channel, Outgoing};
use crate::connection::{CallError, GuestConnection};
use crate::session_core::{Action, BasicChecks, Event, SessionChecks, SessionCore};
use crate::{ChannelState, HostError, MaybeSend, Negotiated};
use async_trait::async_trait;
use morphir_extension_sdk::protocol::{ExtensionNotification, InitializeParams, methods};
use serde_json::Value;

/// A guest reached over JSON-RPC messages.
///
/// The connection owns the MEP codec and the session core. The channel only
/// moves messages.
pub struct JsonRpcConnection<C, K: SessionChecks<Error = HostError> = BasicChecks> {
    channel: C,
    core: SessionCore<K>,
    finished: bool,
}

impl<C: Channel, K: SessionChecks<Error = HostError> + MaybeSend> JsonRpcConnection<C, K> {
    /// A connection that has not run the handshake.
    pub fn new(channel: C, checks: K) -> Self {
        Self {
            channel,
            core: SessionCore::new(checks),
            finished: false,
        }
    }

    /// Feed one event to the core, and exchange messages until it needs the caller.
    ///
    /// A transport failure returns `Err`. The channel already failed, so it is
    /// not closed again.
    async fn step(&mut self, event: Event) -> Result<Action<HostError>, HostError> {
        let mut action = self.core.handle(event);
        while let Action::Send(request) = action {
            if let Err(error) = self.channel.send(Outgoing::Request(request)).await {
                self.core.handle(Event::TransportFailed);
                self.finished = true;
                return Err(error.into());
            }
            match self.channel.receive().await {
                Ok(response) => action = self.core.handle(Event::Received(response)),
                Err(error) => {
                    self.core.handle(Event::TransportFailed);
                    self.finished = true;
                    return Err(error.into());
                }
            }
        }
        Ok(action)
    }

    /// Abort the channel after a protocol failure, and keep the first error.
    async fn abort(&mut self, error: HostError) -> HostError {
        let result = self.channel.close().await;
        self.finished = true;
        match result {
            Ok(_) => error,
            Err(close) => HostError::Channel {
                message: format!("{error}; transport abort also failed: {}", close.message),
                state: close.state,
            },
        }
    }

    fn unexpected(action: &Action<HostError>) -> HostError {
        HostError::Invalid(format!(
            "Session core returned an unexpected action: {action:?}"
        ))
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<C: Channel, K: SessionChecks<Error = HostError> + MaybeSend> GuestConnection
    for JsonRpcConnection<C, K>
{
    async fn open(&mut self, params: InitializeParams) -> Result<Negotiated, HostError> {
        if self.finished {
            return Err(HostError::State {
                action: "open",
                state: "closed",
            });
        }
        match self.step(Event::Open(params)).await? {
            Action::Ready => Ok(self
                .core
                .negotiated()
                .expect("a ready session is negotiated")
                .clone()),
            Action::Failed(error) | Action::Rejected(error) => Err(self.abort(error).await),
            other => {
                let error = Self::unexpected(&other);
                Err(self.abort(error).await)
            }
        }
    }

    async fn call(&mut self, method: &str, params: Value) -> Result<Value, CallError> {
        if self.finished {
            return Err(CallError::Failed(HostError::State {
                action: "call",
                state: "closed",
            }));
        }
        let event = Event::Call {
            method: method.to_owned(),
            params,
        };
        match self.step(event).await {
            Ok(Action::Completed(value)) => Ok(value),
            Ok(Action::Rejected(error)) => Err(CallError::Rejected(error)),
            Ok(Action::Failed(error)) => Err(CallError::Failed(self.abort(error).await)),
            Ok(other) => {
                let error = Self::unexpected(&other);
                Err(CallError::Failed(self.abort(error).await))
            }
            Err(error) => Err(CallError::Failed(error)),
        }
    }

    async fn close(&mut self) -> Result<(), HostError> {
        if self.finished {
            return Ok(());
        }
        match self.step(Event::Close).await? {
            Action::ShutDown => {
                if let Err(error) = self
                    .channel
                    .send(Outgoing::Notification(
                        ExtensionNotification::without_params(methods::EXIT),
                    ))
                    .await
                {
                    self.finished = true;
                    return Err(error.into());
                }
                let result = self.channel.close().await;
                self.finished = true;
                match result? {
                    ChannelState::Stopped => Ok(()),
                    ChannelState::Indeterminate => Err(HostError::Channel {
                        message: "Extension shutdown outcome is indeterminate".into(),
                        state: ChannelState::Indeterminate,
                    }),
                }
            }
            Action::Failed(error) | Action::Rejected(error) => Err(self.abort(error).await),
            other => {
                let error = Self::unexpected(&other);
                Err(self.abort(error).await)
            }
        }
    }
}

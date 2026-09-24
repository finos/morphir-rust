//! A `morphir_host::Channel` over a child process's standard streams.

use crate::process::child::ProcessChild;
use crate::process::launch::ProcessLaunch;
use async_trait::async_trait;
use morphir_extension_sdk::protocol::ExtensionResponse;
use morphir_host::{Channel, ChannelError, ChannelState, ExpectedExtension, HostError, Outgoing};

/// A guest that runs as a child process and speaks MEP over stdio.
///
/// `close` does not send `morphir.exit`: the connection sends it through
/// `send` first. When `send`, `receive` or `close` fails, the channel kills
/// the child before it returns the error. The error's state is `Stopped`
/// when the kill succeeded and `Indeterminate` when it failed too.
pub struct ProcessChannel {
    child: ProcessChild,
    expectation: ExpectedExtension,
    method: Option<String>,
}

impl ProcessChannel {
    /// Start the process that `launch` describes.
    pub async fn spawn(launch: ProcessLaunch) -> Result<Self, HostError> {
        let child = ProcessChild::spawn(&launch).await?;
        Ok(Self {
            child,
            expectation: launch.expectation(),
            method: None,
        })
    }

    /// What the host should expect of the guest this channel started.
    pub fn expectation(&self) -> ExpectedExtension {
        self.expectation.clone()
    }

    /// Standard error collected after the guest exited.
    pub fn stderr_output(&self) -> &str {
        self.child.stderr_output()
    }

    /// Kill the child after `error`, and report what that proves.
    async fn fail(&mut self, error: HostError) -> ChannelError {
        match self.child.abort().await {
            Ok(()) => ChannelError {
                message: error.to_string(),
                state: ChannelState::Stopped,
            },
            Err(cleanup) => ChannelError {
                message: format!("{error}; process cleanup also failed: {cleanup}"),
                state: ChannelState::Indeterminate,
            },
        }
    }

    /// Give a timeout the text of the request or notification it hit.
    fn name_timeout(&self, error: HostError, subject: impl FnOnce() -> String) -> HostError {
        match error {
            HostError::Channel { state, .. } => HostError::Channel {
                message: format!(
                    "{} timed out after {:?}",
                    subject(),
                    self.child.request_timeout()
                ),
                state,
            },
            other => other,
        }
    }

    fn request_subject(&self) -> String {
        format!(
            "Extension request '{}'",
            self.method.as_deref().unwrap_or_default()
        )
    }
}

#[async_trait]
impl Channel for ProcessChannel {
    async fn send(&mut self, message: Outgoing) -> Result<(), ChannelError> {
        let result = match &message {
            Outgoing::Request(request) => {
                self.method = Some(request.method.clone());
                self.child.write(request).await
            }
            Outgoing::Notification(notification) => self.child.write(notification).await,
        };
        let Err(error) = result else {
            return Ok(());
        };
        let error = match &message {
            Outgoing::Request(_) => self.name_timeout(error, || self.request_subject()),
            Outgoing::Notification(notification) => self.name_timeout(error, || {
                let method = notification.method.as_str();
                let name = method.strip_prefix("morphir.").unwrap_or(method);
                format!("Extension {name} notification")
            }),
        };
        Err(self.fail(error).await)
    }

    async fn receive(&mut self) -> Result<ExtensionResponse, ChannelError> {
        let response = match self.child.read().await {
            Ok(frame) => {
                serde_json::from_slice::<ExtensionResponse>(&frame).map_err(HostError::from)
            }
            Err(error) => Err(self.name_timeout(error, || self.request_subject())),
        };
        match response {
            Ok(response) => Ok(response),
            Err(error) => Err(self.fail(error).await),
        }
    }

    async fn close(&mut self) -> Result<ChannelState, ChannelError> {
        match self.child.wait_for_exit().await {
            Ok(status) if status.success() => Ok(ChannelState::Stopped),
            Ok(status) => Err(ChannelError {
                message: format!("Extension process exited with status {status}"),
                state: ChannelState::Stopped,
            }),
            Err(error) => Err(self.fail(error).await),
        }
    }

    async fn abort(&mut self) -> Result<ChannelState, ChannelError> {
        self.child
            .abort()
            .await
            .map(|()| ChannelState::Stopped)
            .map_err(|error| ChannelError {
                message: error.to_string(),
                state: ChannelState::Indeterminate,
            })
    }
}

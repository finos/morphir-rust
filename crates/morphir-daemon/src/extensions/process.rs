//! Native child-process transport for the Morphir Extension Protocol.

mod describe;
mod transport;

pub use describe::{DescriptionSource, ProcessDescription};
pub use morphir_host_native::process::ProcessLaunch;
pub use transport::SpawnedProcessTransport;

#[cfg(test)]
mod tests;

use crate::extensions::protocol::{
    ExtensionNotification, ExtensionRequest, ExtensionResponse, ExtensionResponseExt,
    InitializeParams, InitializeResult, error_codes, methods,
};
use crate::extensions::session::{
    ExpectedExtension, ExtensionSession, ExtensionSessionState, Loaded, MepTransport,
    NegotiatedSession, Session, Stopped, TransportError, TransportState, validate_negotiation,
};
use crate::{DaemonError, Result};
use async_trait::async_trait;
use morphir_host::HostError;
use morphir_host_native::process::ProcessChild;
use serde::{Serialize, de::DeserializeOwned};

enum ProcessSessionData {
    Starting,
    Ready(Box<CompatibilityReady>),
    Stopped,
}

struct CompatibilityReady {
    negotiated: NegotiatedSession,
}

/// A runtime-erased MEP session carried over a child process's standard streams.
///
/// Compatibility sessions cannot be reused as typestate transports after their
/// lifecycle has started.
///
/// ```compile_fail
/// use morphir_daemon::extensions::{Session, SpawnedProcessSession};
/// use morphir_extension_sdk::protocol::InitializeParams;
/// fn cannot_rewrap(session: SpawnedProcessSession, params: InitializeParams) {
///     let _initialization = Session::loaded(session).initialize(params);
/// }
/// ```
pub struct SpawnedProcessSession {
    expected_extension: ExpectedExtension,
    child: ProcessChild,
    next_request_id: u64,
    state: ProcessSessionData,
}

impl SpawnedProcessSession {
    /// Start a native extension and connect its standard streams.
    pub async fn spawn(launch: ProcessLaunch) -> Result<Self> {
        let child = ProcessChild::spawn(&launch).await?;
        Ok(Self {
            expected_extension: launch.expectation(),
            child,
            next_request_id: 1,
            state: ProcessSessionData::Starting,
        })
    }

    /// Start a native extension behind the shared typestate session controller.
    pub async fn spawn_typestate(
        launch: ProcessLaunch,
    ) -> Result<Session<SpawnedProcessTransport, Loaded>> {
        Ok(Session::loaded(
            SpawnedProcessTransport::spawn(launch).await?,
        ))
    }

    /// Return captured standard error after the process exits.
    pub fn stderr_output(&self) -> &str {
        self.child.stderr_output()
    }

    /// Report whether the child process is still running.
    pub fn is_running(&mut self) -> Result<bool> {
        self.child.is_running().map_err(DaemonError::from)
    }

    fn ready_session(&self) -> Result<&CompatibilityReady> {
        match &self.state {
            ProcessSessionData::Ready(initialized) => Ok(initialized),
            ProcessSessionData::Starting | ProcessSessionData::Stopped => Err(
                DaemonError::Extension("Extension session is not ready".to_string()),
            ),
        }
    }

    fn expected_extension(&self) -> ExpectedExtension {
        self.expected_extension.clone()
    }

    async fn call<P, R>(&mut self, method: &str, params: P) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.checked_add(1).ok_or_else(|| {
            DaemonError::Extension("Extension request identifier overflowed".to_string())
        })?;
        let request = ExtensionRequest::new(method, params, request_id)?;

        let response = match self.child.exchange(&request).await {
            Ok(frame) => {
                serde_json::from_slice::<ExtensionResponse>(&frame).map_err(DaemonError::from)
            }
            Err(error) => Err(DaemonError::from(error)),
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => return Err(self.abort_with_error(error).await),
        };

        if let Err(error) = response.validate_envelope(request_id) {
            return Err(self.abort_with_error(error).await);
        }
        response.into_result(request_id)
    }

    async fn send_exit_notification(&mut self) -> Result<()> {
        let notification = ExtensionNotification::without_params(methods::EXIT);
        match self.child.write(&notification).await {
            Ok(()) => Ok(()),
            Err(HostError::Channel { .. }) => Err(DaemonError::Extension(format!(
                "Extension exit notification timed out after {:?}",
                self.child.request_timeout()
            ))),
            Err(error) => Err(error.into()),
        }
    }

    async fn abort_with_error(&mut self, error: DaemonError) -> DaemonError {
        match self.abort_process().await {
            Ok(()) => error,
            Err(cleanup_error) => DaemonError::Extension(format!(
                "{}; process cleanup also failed: {}",
                error, cleanup_error
            )),
        }
    }

    async fn abort_process(&mut self) -> Result<()> {
        self.child.abort().await?;
        self.state = ProcessSessionData::Stopped;
        Ok(())
    }
}

#[async_trait]
impl ExtensionSession for SpawnedProcessSession {
    fn state(&self) -> ExtensionSessionState {
        match self.state {
            ProcessSessionData::Starting => ExtensionSessionState::Starting,
            ProcessSessionData::Ready(_) => ExtensionSessionState::Ready,
            ProcessSessionData::Stopped => ExtensionSessionState::Stopped,
        }
    }

    async fn initialize(&mut self, params: InitializeParams) -> Result<InitializeResult> {
        if !matches!(self.state, ProcessSessionData::Starting) {
            return Err(DaemonError::Extension(
                "Extension session can only initialize once".to_string(),
            ));
        }

        let offered_versions = params.protocol_versions.clone();
        let initialized: InitializeResult = self.call(methods::INITIALIZE, params).await?;
        let negotiated = validate_compatibility_initialization(
            self.expected_extension(),
            &offered_versions,
            initialized.clone(),
        );
        let negotiated = match negotiated {
            Ok(negotiated) => negotiated,
            Err(error) => return Err(self.abort_with_error(error).await),
        };

        self.state = ProcessSessionData::Ready(Box::new(CompatibilityReady { negotiated }));
        Ok(initialized)
    }

    async fn invoke(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let ready = self.ready_session()?;
        if matches!(
            method,
            methods::INITIALIZE | methods::SHUTDOWN | methods::EXIT
        ) {
            return Err(DaemonError::Extension(format!(
                "Protocol lifecycle method '{}' must use its dedicated session operation",
                method
            )));
        }
        if !ready.negotiated.supports_method(method) {
            return Err(DaemonError::Extension(format!(
                "RPC error {}: Extension '{}' does not support capability '{}'",
                error_codes::CAPABILITY_UNAVAILABLE,
                ready.negotiated.extension().id,
                method
            )));
        }
        if !ready.negotiated.supports_invocation(method, &params) {
            return Err(DaemonError::Extension(format!(
                "RPC error {}: Extension '{}' does not support capability '{}' for the requested protocol",
                error_codes::CAPABILITY_UNAVAILABLE,
                ready.negotiated.extension().id,
                method
            )));
        }

        let request_params = params.clone();
        let value: serde_json::Value = self.call(method, params).await?;
        match validate_compatibility_method_result(method, request_params, value).await {
            Ok(value) => Ok(value),
            Err(error) => Err(self.abort_with_error(error).await),
        }
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.ready_session()?;
        let _: serde_json::Value = self.call(methods::SHUTDOWN, serde_json::json!({})).await?;
        if let Err(error) = self.send_exit_notification().await {
            return Err(self.abort_with_error(error).await);
        }
        let status = match self.child.wait_for_status().await {
            Ok(status) => status,
            Err(error @ HostError::Channel { .. }) => {
                return Err(self.abort_with_error(error.into()).await);
            }
            Err(error) => return Err(error.into()),
        };
        self.child.collect_stderr().await?;
        self.state = ProcessSessionData::Stopped;
        if !status.success() {
            return Err(DaemonError::Extension(format!(
                "Extension process exited with status {status}"
            )));
        }

        Ok(())
    }
}

fn validate_compatibility_initialization(
    expected: ExpectedExtension,
    offered_versions: &[String],
    initialized: InitializeResult,
) -> Result<NegotiatedSession> {
    validate_negotiation(expected, offered_versions, initialized)
}

async fn validate_compatibility_method_result(
    method: &str,
    request_params: serde_json::Value,
    value: serde_json::Value,
) -> Result<serde_json::Value> {
    morphir_host_native::validate_result(method, request_params, value)
        .await
        .map_err(DaemonError::from)
}

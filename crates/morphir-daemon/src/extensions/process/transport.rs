use super::*;

/// Fresh child-process transport owned by a typestate session.
///
/// Only [`SpawnedProcessSession::spawn_typestate`] constructs this type, so a
/// runtime-erased compatibility session cannot be reintroduced as loaded.
pub struct SpawnedProcessTransport {
    pub(super) session: SpawnedProcessSession,
}

impl SpawnedProcessTransport {
    /// Start a native extension as a fresh shared MEP transport.
    pub async fn spawn(launch: ProcessLaunch) -> Result<Self> {
        Ok(Self {
            session: SpawnedProcessSession::spawn(launch).await?,
        })
    }
}

#[async_trait]
impl MepTransport for SpawnedProcessTransport {
    fn expected_extension(&self) -> ExpectedExtension {
        self.session.expected_extension()
    }

    async fn exchange(
        &mut self,
        request: ExtensionRequest,
    ) -> std::result::Result<ExtensionResponse, TransportError> {
        let result = match self.session.child.exchange(&request).await {
            Ok(frame) => {
                serde_json::from_slice::<ExtensionResponse>(&frame).map_err(DaemonError::from)
            }
            Err(error) => Err(DaemonError::from(error)),
        };
        match result {
            Ok(response) => Ok(response),
            Err(error) => Err(self.stop_after(error).await),
        }
    }

    async fn abort(&mut self) -> std::result::Result<TransportState, TransportError> {
        self.session
            .abort_process()
            .await
            .map(|()| TransportState::Stopped)
            .map_err(|error| TransportError::new(error, TransportState::Indeterminate))
    }

    async fn terminate(&mut self) -> std::result::Result<TransportState, TransportError> {
        if let Err(error) = self.session.send_exit_notification().await {
            return Err(self.stop_after(error).await);
        }
        self.finish_after_exit().await
    }
}

impl SpawnedProcessTransport {
    /// Wait for the process to exit after `exit` was sent, and collect stderr.
    ///
    /// A process that outlives the request timeout is killed.
    pub(super) async fn finish_after_exit(
        &mut self,
    ) -> std::result::Result<TransportState, TransportError> {
        let status = match self.session.child.wait_for_status().await {
            Ok(status) => status,
            Err(error @ HostError::Channel { .. }) => {
                return Err(self.stop_after(error.into()).await);
            }
            Err(error) => {
                return Err(TransportError::new(
                    error.into(),
                    TransportState::Indeterminate,
                ));
            }
        };
        self.session
            .child
            .collect_stderr()
            .await
            .map_err(|error| TransportError::new(error.into(), TransportState::Stopped))?;
        self.session.state = ProcessSessionData::Stopped;
        if !status.success() {
            return Err(TransportError::new(
                DaemonError::Extension(format!("Extension process exited with status {status}")),
                TransportState::Stopped,
            ));
        }
        Ok(TransportState::Stopped)
    }

    /// Kill the process after `error`, and report what that proves.
    pub(super) async fn stop_after(&mut self, error: DaemonError) -> TransportError {
        match self.session.abort_process().await {
            Ok(()) => TransportError::new(error, TransportState::Stopped),
            Err(cleanup) => TransportError::new(
                DaemonError::Extension(format!("{error}; process cleanup also failed: {cleanup}")),
                TransportState::Indeterminate,
            ),
        }
    }
}

impl<S> Session<SpawnedProcessTransport, S> {
    /// Report whether the child process is still running without exposing transport I/O.
    pub fn process_is_running(&mut self) -> Result<bool> {
        self.transport_mut_internal().session.is_running()
    }

    /// Return captured child-process diagnostics without exposing transport I/O.
    pub fn process_stderr_output(&self) -> &str {
        self.transport_internal().session.stderr_output()
    }
}

impl Session<SpawnedProcessTransport, Stopped> {
    /// Report whether the stopped child left any unread bytes on standard output.
    ///
    /// A conforming process writes only the response frames consumed by the host.
    /// Remaining bytes therefore identify protocol output that was not framed as a
    /// response.
    pub async fn process_stdout_is_exhausted(&mut self) -> Result<bool> {
        self.transport_mut_internal()
            .session
            .child
            .stdout_is_exhausted()
            .await
            .map_err(Into::into)
    }
}

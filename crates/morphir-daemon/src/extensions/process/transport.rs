use super::*;

/// Fresh child-process transport owned by a typestate session.
///
/// Only [`SpawnedProcessSession::spawn_typestate`] constructs this type, so a
/// runtime-erased compatibility session cannot be reintroduced as loaded.
pub struct SpawnedProcessTransport {
    session: SpawnedProcessSession,
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
        let method = request.method.clone();
        let exchange = async {
            let stdin = self.session.stdin.as_mut().ok_or_else(|| {
                DaemonError::Extension("Extension process stdin is closed".to_string())
            })?;
            write_frame(stdin, &request).await?;
            let frame = read_frame(&mut self.session.stdout).await?;
            serde_json::from_slice::<ExtensionResponse>(&frame).map_err(DaemonError::from)
        };
        let result = match timeout(self.session.request_timeout, exchange).await {
            Ok(result) => result,
            Err(_) => Err(DaemonError::Extension(format!(
                "Extension request '{}' timed out after {:?}",
                method, self.session.request_timeout
            ))),
        };
        match result {
            Ok(response) => Ok(response),
            Err(error) => Err(match self.session.abort_process().await {
                Ok(()) => TransportError::new(error, TransportState::Stopped),
                Err(cleanup) => TransportError::new(
                    DaemonError::Extension(format!(
                        "{error}; process cleanup also failed: {cleanup}"
                    )),
                    TransportState::Indeterminate,
                ),
            }),
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
            return Err(match self.session.abort_process().await {
                Ok(()) => TransportError::new(error, TransportState::Stopped),
                Err(cleanup) => TransportError::new(
                    DaemonError::Extension(format!(
                        "{error}; process cleanup also failed: {cleanup}"
                    )),
                    TransportState::Indeterminate,
                ),
            });
        }
        self.session.stdin.take();
        let status = match timeout(self.session.request_timeout, self.session.child.wait()).await {
            Ok(status) => status.map_err(|error| {
                TransportError::new(error.into(), TransportState::Indeterminate)
            })?,
            Err(_) => {
                let error = DaemonError::Extension(format!(
                    "Extension process did not exit after {:?}",
                    self.session.request_timeout
                ));
                return Err(match self.session.abort_process().await {
                    Ok(()) => TransportError::new(error, TransportState::Stopped),
                    Err(cleanup) => TransportError::new(
                        DaemonError::Extension(format!(
                            "{error}; process cleanup also failed: {cleanup}"
                        )),
                        TransportState::Indeterminate,
                    ),
                });
            }
        };
        self.session
            .collect_stderr()
            .await
            .map_err(|error| TransportError::new(error, TransportState::Stopped))?;
        self.session.state = ProcessSessionData::Stopped;
        if !status.success() {
            return Err(TransportError::new(
                DaemonError::Extension(format!("Extension process exited with status {status}")),
                TransportState::Stopped,
            ));
        }
        Ok(TransportState::Stopped)
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
        let request_timeout = self.transport_internal().session.request_timeout;
        let stdout = &mut self.transport_mut_internal().session.stdout;
        match timeout(request_timeout, stdout.fill_buf()).await {
            Ok(result) => result
                .map(|remaining| remaining.is_empty())
                .map_err(Into::into),
            Err(_) => Err(DaemonError::Extension(format!(
                "Timed out while checking extension stdout after {request_timeout:?}"
            ))),
        }
    }
}

//! Native child-process transport for the Morphir Extension Protocol.

use crate::extensions::protocol::{
    ExtensionNotification, ExtensionRequest, ExtensionResponse, ExtensionResponseExt,
    InitializeParams, InitializeResult, MAX_MEP_PAYLOAD_BYTES, error_codes, methods,
};
use crate::extensions::session::{
    CapabilityExpectation, ExpectedExtension, ExtensionSession, ExtensionSessionState, Loaded,
    MepTransport, NegotiatedSession, Session, Stopped, TransportError, TransportState,
    validate_negotiation,
};
use crate::{DaemonError, Result};
use async_trait::async_trait;
use morphir_extension_sdk::{BackendCapability, ExtensionCapabilities, ExtensionInfo};
use serde::{Serialize, de::DeserializeOwned};
use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::task::JoinHandle;
use tokio::time::timeout;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_STDERR_BYTES: usize = 256 * 1024;
const EXECUTABLE_BUSY_RETRIES: usize = 4;
const EXECUTABLE_BUSY_RETRY_DELAY: Duration = Duration::from_millis(10);

/// A native extension command with an explicit identity and working directory.
#[derive(Debug, Clone)]
pub struct ProcessLaunch {
    extension_id: String,
    discovered: Option<ExtensionInfo>,
    capabilities: Option<CapabilityExpectation>,
    allows_legacy_backend: bool,
    program: ProcessProgram,
    args: Vec<OsString>,
    working_directory: PathBuf,
    environment: Vec<(OsString, OsString)>,
    request_timeout: Duration,
}

#[derive(Debug, Clone)]
enum ProcessProgram {
    Path(PathBuf),
    VerifiedBytes {
        filename: OsString,
        bytes: Arc<[u8]>,
        staging_directory: Option<PathBuf>,
    },
}

impl ProcessLaunch {
    /// Define a process launch without inheriting the host environment.
    pub fn new(
        extension_id: impl Into<String>,
        program: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: extension_id.into(),
            discovered: None,
            capabilities: None,
            allows_legacy_backend: false,
            program: ProcessProgram::Path(program.into()),
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Define a verified process launch with exact discovery metadata.
    ///
    /// Typestate initialization requires the child to reproduce this identity,
    /// name, version, and capability set. Use [`Self::new`] for explicit
    /// development commands that have only a configured identity.
    pub fn from_discovered(
        discovered: ExtensionInfo,
        program: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: discovered.id.clone(),
            discovered: Some(discovered),
            capabilities: None,
            allows_legacy_backend: false,
            program: ProcessProgram::Path(program.into()),
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Define a verified process launch with exact discovery capabilities.
    pub fn from_discovered_capabilities(
        discovered: ExtensionInfo,
        capabilities: ExtensionCapabilities,
        program: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: discovered.id.clone(),
            discovered: Some(discovered),
            capabilities: Some(CapabilityExpectation::Exact(capabilities)),
            allows_legacy_backend: false,
            program: ProcessProgram::Path(program.into()),
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Define a verified process launch from immutable executable bytes.
    pub fn from_verified_bytes(
        discovered: ExtensionInfo,
        filename: &OsStr,
        bytes: &[u8],
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: discovered.id.clone(),
            discovered: Some(discovered),
            capabilities: None,
            allows_legacy_backend: false,
            program: ProcessProgram::VerifiedBytes {
                filename: filename.to_os_string(),
                bytes: Arc::from(bytes),
                staging_directory: None,
            },
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Define a verified process launch staged below an explicit executable directory.
    pub fn from_verified_bytes_in(
        discovered: ExtensionInfo,
        filename: &OsStr,
        bytes: &[u8],
        staging_directory: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: discovered.id.clone(),
            discovered: Some(discovered),
            capabilities: None,
            allows_legacy_backend: false,
            program: ProcessProgram::VerifiedBytes {
                filename: filename.to_os_string(),
                bytes: Arc::from(bytes),
                staging_directory: Some(staging_directory.into()),
            },
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Define a verified process launch from immutable bytes and capabilities.
    pub fn from_verified_bytes_with_capabilities(
        discovered: ExtensionInfo,
        capabilities: ExtensionCapabilities,
        filename: &OsStr,
        bytes: &[u8],
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: discovered.id.clone(),
            discovered: Some(discovered),
            capabilities: Some(CapabilityExpectation::Exact(capabilities)),
            allows_legacy_backend: false,
            program: ProcessProgram::VerifiedBytes {
                filename: filename.to_os_string(),
                bytes: Arc::from(bytes),
                staging_directory: None,
            },
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Define a capability-locked verified launch below an executable directory.
    pub fn from_verified_bytes_with_capabilities_in(
        discovered: ExtensionInfo,
        capabilities: ExtensionCapabilities,
        filename: &OsStr,
        bytes: &[u8],
        staging_directory: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: discovered.id.clone(),
            discovered: Some(discovered),
            capabilities: Some(CapabilityExpectation::Exact(capabilities)),
            allows_legacy_backend: false,
            program: ProcessProgram::VerifiedBytes {
                filename: filename.to_os_string(),
                bytes: Arc::from(bytes),
                staging_directory: Some(staging_directory.into()),
            },
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Define a backend-locked verified launch below an executable directory.
    pub(crate) fn from_verified_bytes_with_backend_capability_in(
        discovered: ExtensionInfo,
        backend: BackendCapability,
        filename: &OsStr,
        bytes: &[u8],
        staging_directory: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: discovered.id.clone(),
            discovered: Some(discovered),
            capabilities: Some(CapabilityExpectation::Backend(backend)),
            allows_legacy_backend: false,
            program: ProcessProgram::VerifiedBytes {
                filename: filename.to_os_string(),
                bytes: Arc::from(bytes),
                staging_directory: Some(staging_directory.into()),
            },
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Define a verified schema-v1 process launch with legacy backend behavior.
    pub(crate) fn from_legacy_verified_bytes_in(
        discovered: ExtensionInfo,
        filename: &OsStr,
        bytes: &[u8],
        staging_directory: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        let mut launch = Self::from_verified_bytes_in(
            discovered,
            filename,
            bytes,
            staging_directory,
            working_directory,
        );
        launch.allows_legacy_backend = true;
        launch
    }

    /// Append one process argument.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Add one environment variable to the otherwise empty child environment.
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.environment.push((key.into(), value.into()));
        self
    }

    /// Set the timeout applied to each request and to process shutdown.
    pub fn request_timeout(mut self, request_timeout: Duration) -> Self {
        self.request_timeout = request_timeout;
        self
    }
}

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
    expected_extension_id: String,
    discovered: Option<ExtensionInfo>,
    capabilities: Option<CapabilityExpectation>,
    allows_legacy_backend: bool,
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr_task: Option<JoinHandle<std::io::Result<Vec<u8>>>>,
    stderr_output: String,
    next_request_id: u64,
    request_timeout: Duration,
    state: ProcessSessionData,
    _staged_program: Option<tempfile::TempDir>,
}

impl SpawnedProcessSession {
    /// Start a native extension and connect its standard streams.
    pub async fn spawn(launch: ProcessLaunch) -> Result<Self> {
        validate_launch(&launch)?;

        let (program, staged_program) = prepare_program(&launch.program).await?;
        let mut command = Command::new(&program);
        command
            .args(&launch.args)
            .current_dir(&launch.working_directory)
            .env_clear()
            .envs(launch.environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = spawn_child(&mut command).await.map_err(|error| {
            DaemonError::Extension(format!(
                "Failed to start extension '{}': {}",
                launch.extension_id, error
            ))
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            DaemonError::Extension("Extension process stdin was not captured".to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            DaemonError::Extension("Extension process stdout was not captured".to_string())
        })?;
        let mut stderr = child.stderr.take().ok_or_else(|| {
            DaemonError::Extension("Extension process stderr was not captured".to_string())
        })?;
        let stderr_task = tokio::spawn(async move { read_bounded_tail(&mut stderr).await });

        Ok(Self {
            expected_extension_id: launch.extension_id,
            discovered: launch.discovered,
            capabilities: launch.capabilities,
            allows_legacy_backend: launch.allows_legacy_backend,
            child,
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
            stderr_task: Some(stderr_task),
            stderr_output: String::new(),
            next_request_id: 1,
            request_timeout: launch.request_timeout,
            state: ProcessSessionData::Starting,
            _staged_program: staged_program,
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
        &self.stderr_output
    }

    /// Report whether the child process is still running.
    pub fn is_running(&mut self) -> Result<bool> {
        self.child
            .try_wait()
            .map(|status| status.is_none())
            .map_err(DaemonError::from)
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
        match (&self.discovered, &self.capabilities) {
            (Some(discovered), Some(capabilities)) => {
                ExpectedExtension::discovered_with_expectation(
                    discovered.clone(),
                    capabilities.clone(),
                )
            }
            (Some(discovered), None) if self.allows_legacy_backend => {
                ExpectedExtension::legacy_discovered(discovered.clone())
            }
            (Some(discovered), None) => ExpectedExtension::discovered(discovered.clone()),
            (None, _) => ExpectedExtension::identified(self.expected_extension_id.clone()),
        }
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

        let exchange = async {
            let stdin = self.stdin.as_mut().ok_or_else(|| {
                DaemonError::Extension("Extension process stdin is closed".to_string())
            })?;
            write_frame(stdin, &request).await?;
            let frame = read_frame(&mut self.stdout).await?;
            serde_json::from_slice::<ExtensionResponse>(&frame).map_err(DaemonError::from)
        };
        let response = match timeout(self.request_timeout, exchange).await {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                return Err(self.abort_with_error(error).await);
            }
            Err(_) => {
                let error = DaemonError::Extension(format!(
                    "Extension request '{}' timed out after {:?}",
                    method, self.request_timeout
                ));
                return Err(self.abort_with_error(error).await);
            }
        };

        if let Err(error) = response.validate_envelope(request_id) {
            return Err(self.abort_with_error(error).await);
        }
        response.into_result(request_id)
    }

    async fn collect_stderr(&mut self) -> Result<()> {
        let Some(mut stderr_task) = self.stderr_task.take() else {
            return Ok(());
        };
        match timeout(self.request_timeout, &mut stderr_task).await {
            Ok(result) => {
                let output = result.map_err(|error| {
                    DaemonError::Extension(format!(
                        "Failed to join extension stderr reader: {error}"
                    ))
                })??;
                self.stderr_output = String::from_utf8_lossy(&output).into_owned();
            }
            Err(_) => {
                stderr_task.abort();
                let _ = stderr_task.await;
            }
        }
        Ok(())
    }

    async fn send_exit_notification(&mut self) -> Result<()> {
        let notification = ExtensionNotification::without_params(methods::EXIT);
        let send = async {
            let stdin = self.stdin.as_mut().ok_or_else(|| {
                DaemonError::Extension("Extension process stdin is closed".to_string())
            })?;
            write_frame(stdin, &notification).await
        };
        match timeout(self.request_timeout, send).await {
            Ok(result) => result,
            Err(_) => Err(DaemonError::Extension(format!(
                "Extension exit notification timed out after {:?}",
                self.request_timeout
            ))),
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
        self.stdin.take();
        if self.child.try_wait()?.is_none() {
            self.child.kill().await?;
        }
        let _ = self.child.wait().await?;
        self.cancel_stderr();
        self.state = ProcessSessionData::Stopped;
        Ok(())
    }

    fn cancel_stderr(&mut self) {
        if let Some(stderr_task) = self.stderr_task.take() {
            stderr_task.abort();
        }
    }
}

async fn spawn_child(command: &mut Command) -> std::io::Result<Child> {
    let mut retries = 0;
    loop {
        match command.spawn() {
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && retries < EXECUTABLE_BUSY_RETRIES =>
            {
                retries += 1;
                tokio::time::sleep(EXECUTABLE_BUSY_RETRY_DELAY).await;
            }
            result => return result,
        }
    }
}

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

        self.call(method, params).await
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.ready_session()?;
        let _: serde_json::Value = self.call(methods::SHUTDOWN, serde_json::json!({})).await?;
        if let Err(error) = self.send_exit_notification().await {
            return Err(self.abort_with_error(error).await);
        }
        self.stdin.take();

        let status = match timeout(self.request_timeout, self.child.wait()).await {
            Ok(status) => status?,
            Err(_) => {
                let error = DaemonError::Extension(format!(
                    "Extension process did not exit after {:?}",
                    self.request_timeout
                ));
                return Err(self.abort_with_error(error).await);
            }
        };
        self.collect_stderr().await?;
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

async fn read_bounded_tail(reader: &mut (impl AsyncRead + Unpin)) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut chunk = [0; 8 * 1024];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            return Ok(output);
        }
        append_bounded_tail(&mut output, &chunk[..read], MAX_STDERR_BYTES);
    }
}

fn append_bounded_tail(output: &mut Vec<u8>, chunk: &[u8], limit: usize) {
    if chunk.len() >= limit {
        output.clear();
        output.extend_from_slice(&chunk[chunk.len() - limit..]);
        return;
    }

    let excess = output
        .len()
        .saturating_add(chunk.len())
        .saturating_sub(limit);
    if excess > 0 {
        output.drain(..excess);
    }
    output.extend_from_slice(chunk);
}

fn validate_launch(launch: &ProcessLaunch) -> Result<()> {
    if launch.extension_id.trim().is_empty() {
        return Err(DaemonError::Extension(
            "Extension process identity cannot be empty".to_string(),
        ));
    }
    if let ProcessProgram::Path(program) = &launch.program
        && !program.is_file()
    {
        return Err(DaemonError::Extension(format!(
            "Extension executable does not exist: {}",
            program.display()
        )));
    }
    if !launch.working_directory.is_dir() {
        return Err(DaemonError::Extension(format!(
            "Extension working directory does not exist: {}",
            launch.working_directory.display()
        )));
    }
    Ok(())
}

async fn prepare_program(program: &ProcessProgram) -> Result<(PathBuf, Option<tempfile::TempDir>)> {
    match program {
        ProcessProgram::Path(path) => Ok((path.clone(), None)),
        ProcessProgram::VerifiedBytes {
            filename,
            bytes,
            staging_directory,
        } => {
            let filename = filename.clone();
            let bytes = Arc::clone(bytes);
            let staging_directory = staging_directory.clone();
            tokio::task::spawn_blocking(move || {
                stage_verified_program(filename, bytes, staging_directory)
            })
            .await
            .map_err(|error| {
                DaemonError::Extension(format!("Extension staging worker failed: {error}"))
            })?
        }
    }
}

fn stage_verified_program(
    filename: OsString,
    bytes: Arc<[u8]>,
    staging_directory: Option<PathBuf>,
) -> Result<(PathBuf, Option<tempfile::TempDir>)> {
    validate_verified_program_filename(&filename)?;
    let mut builder = tempfile::Builder::new();
    builder.prefix("morphir-extension-");
    let directory = match staging_directory {
        Some(staging_directory) => {
            fs::create_dir_all(&staging_directory).map_err(DaemonError::from)?;
            builder
                .tempdir_in(staging_directory)
                .map_err(DaemonError::from)?
        }
        None => builder.tempdir().map_err(DaemonError::from)?,
    };
    let path = directory.path().join(filename);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(DaemonError::from)?;
    std::io::Write::write_all(&mut file, &bytes).map_err(DaemonError::from)?;
    file.sync_all().map_err(DaemonError::from)?;
    make_owner_executable(&path)?;
    Ok((path, Some(directory)))
}

fn validate_verified_program_filename(filename: &OsStr) -> Result<()> {
    let mut components = Path::new(filename).components();
    if matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    ) {
        return Ok(());
    }
    Err(DaemonError::Extension(
        "Verified extension executable must be a single filename".to_owned(),
    ))
}

#[cfg(unix)]
fn make_owner_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).map_err(Into::into)
}

#[cfg(not(unix))]
fn make_owner_executable(_path: &Path) -> Result<()> {
    Ok(())
}

async fn write_frame<W, T>(writer: &mut W, value: &T) -> Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let body = serde_json::to_vec(value)?;
    writer
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await?;
    writer.write_all(&body).await?;
    writer.flush().await?;
    Ok(())
}

async fn read_frame<R>(reader: &mut R) -> Result<Vec<u8>>
where
    R: AsyncBufRead + Unpin,
{
    let mut content_length = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).await? == 0 {
            return Err(DaemonError::Extension(
                "Extension process closed stdout before a response frame".to_string(),
            ));
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
        let (name, value) = header.split_once(':').ok_or_else(|| {
            DaemonError::Extension(format!(
                "Invalid extension protocol header: {}",
                header.trim_end()
            ))
        })?;
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(DaemonError::Extension(
                    "Extension frame repeated Content-Length".to_string(),
                ));
            }
            let length = value.trim().parse::<usize>().map_err(|error| {
                DaemonError::Extension(format!("Invalid Content-Length: {error}"))
            })?;
            if length > MAX_MEP_PAYLOAD_BYTES as usize {
                return Err(DaemonError::Extension(format!(
                    "Extension frame exceeds the {} byte limit",
                    MAX_MEP_PAYLOAD_BYTES
                )));
            }
            content_length = Some(length);
        }
    }

    let content_length = content_length.ok_or_else(|| {
        DaemonError::Extension("Extension frame omitted Content-Length".to_string())
    })?;
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body).await?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_extension_sdk::{BackendCapability, ExtensionInfo, ExtensionType};
    use tokio::io::{BufReader, duplex};

    #[test]
    fn discovered_process_launch_retains_exact_negotiation_metadata() {
        let discovered = ExtensionInfo {
            id: "morphir-elm".into(),
            name: "Morphir Elm".into(),
            version: "3.2.1".into(),
            types: vec![ExtensionType::Frontend],
            ..ExtensionInfo::default()
        };
        let launch = ProcessLaunch::from_discovered(
            discovered.clone(),
            "/verified/morphir-elm",
            "/workspace",
        );

        assert_eq!(launch.extension_id, discovered.id);
        let retained = launch
            .discovered
            .expect("verified launches should retain discovery metadata");
        assert_eq!(retained.name, discovered.name);
        assert_eq!(retained.version, discovered.version);
        assert_eq!(retained.types, discovered.types);
    }

    #[test]
    fn compatibility_initialization_rejects_locked_backend_capability_drift() {
        let discovered = ExtensionInfo {
            id: "example".into(),
            name: "Example".into(),
            version: "1.0.0".into(),
            types: vec![ExtensionType::Backend],
            ..ExtensionInfo::default()
        };
        let locked_backend = BackendCapability {
            targets: vec!["avro".into()],
            ir_versions: vec!["3".into(), "4.0.0".into()],
            generate: true,
        };
        let expected = ExpectedExtension::discovered_with_capabilities(
            discovered.clone(),
            ExtensionCapabilities {
                backend: Some(locked_backend.clone()),
                ..ExtensionCapabilities::default()
            },
        );
        let initialized = InitializeResult {
            protocol_version: "0.1".into(),
            extension: discovered,
            capabilities: ExtensionCapabilities {
                backend: Some(BackendCapability {
                    generate: false,
                    ..locked_backend
                }),
                ..ExtensionCapabilities::default()
            },
        };

        let error = validate_compatibility_initialization(expected, &["0.1".into()], initialized)
            .expect_err("compatibility sessions must enforce discovery-time capability locks");

        assert!(
            error
                .to_string()
                .contains("backend capabilities disagreed with discovery"),
            "{error}"
        );
    }

    #[test]
    fn compatibility_negotiation_retains_disabled_generate_support() {
        let extension = ExtensionInfo {
            id: "example".into(),
            name: "Example".into(),
            version: "1.0.0".into(),
            types: vec![ExtensionType::Backend],
            ..ExtensionInfo::default()
        };
        let initialized = InitializeResult {
            protocol_version: "0.1".into(),
            extension: extension.clone(),
            capabilities: ExtensionCapabilities {
                backend: Some(BackendCapability {
                    targets: vec!["avro".into()],
                    ir_versions: vec!["4".into()],
                    generate: false,
                }),
                ..ExtensionCapabilities::default()
            },
        };

        let negotiated = validate_compatibility_initialization(
            ExpectedExtension::discovered(extension),
            &["0.1".into()],
            initialized,
        )
        .unwrap();

        assert!(!negotiated.supports_method(methods::GENERATE));
    }

    #[tokio::test]
    async fn verified_bytes_stage_under_the_explicit_managed_directory() {
        let root = tempfile::tempdir().unwrap();
        let staging_directory = root.path().join("managed-staging");
        fs::create_dir(&staging_directory).unwrap();
        let launch = ProcessLaunch::from_verified_bytes_in(
            ExtensionInfo {
                id: "example".into(),
                ..ExtensionInfo::default()
            },
            OsStr::new("example"),
            b"#!/bin/sh\n",
            &staging_directory,
            root.path(),
        );

        let (program, _retained_directory) = prepare_program(&launch.program).await.unwrap();

        assert!(program.starts_with(&staging_directory));
    }

    #[test]
    fn verified_bytes_reject_path_components_without_writing_outside_staging() {
        let root = tempfile::tempdir().unwrap();
        let staging_directory = root.path().join("managed-staging");
        let absolute_escape = root.path().join("absolute-escape");

        for filename in [
            OsString::from("../relative-escape"),
            OsString::from("nested/escape"),
            absolute_escape.clone().into_os_string(),
        ] {
            let error = stage_verified_program(
                filename,
                Arc::from(&b"#!/bin/sh\n"[..]),
                Some(staging_directory.clone()),
            )
            .expect_err("verified process filename must be a single basename");

            assert!(error.to_string().contains("single filename"), "{error}");
        }
        assert!(!absolute_escape.exists());
        assert!(!staging_directory.join("relative-escape").exists());
        assert!(!staging_directory.join("nested/escape").exists());
    }

    #[tokio::test]
    async fn content_length_frames_round_trip_formatted_json() {
        let (mut writer, reader) = duplex(1024);
        let value = serde_json::json!({ "message": "line one\nline two" });
        let expected = value.clone();
        let writing = tokio::spawn(async move { write_frame(&mut writer, &value).await });
        let body = read_frame(&mut BufReader::new(reader))
            .await
            .expect("the frame should parse");
        writing.await.expect("the writer task should join").unwrap();

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            expected
        );
    }

    #[tokio::test]
    async fn stdout_logs_are_rejected_as_protocol_headers() {
        let (mut writer, reader) = duplex(1024);
        writer.write_all(b"accidental log line\n").await.unwrap();
        drop(writer);

        let error = read_frame(&mut BufReader::new(reader))
            .await
            .expect_err("stdout logs must not be treated as protocol data");
        assert!(
            error
                .to_string()
                .contains("Invalid extension protocol header")
        );
    }

    #[test]
    fn stderr_capture_retains_only_the_bounded_tail() {
        let mut output = b"old diagnostics".to_vec();
        append_bounded_tail(&mut output, b"new diagnostics", 16);

        assert_eq!(output, b"snew diagnostics");

        append_bounded_tail(&mut output, b"0123456789abcdefghijkl", 16);
        assert_eq!(output, b"6789abcdefghijkl");
    }
}

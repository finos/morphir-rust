//! Native child-process transport for the Morphir Extension Protocol.

use crate::extensions::protocol::{
    ExtensionRequest, ExtensionResponse, ExtensionResponseExt, InitializeParams, InitializeResult,
    MAX_MEP_PAYLOAD_BYTES, error_codes, methods,
};
use crate::extensions::session::{
    ExpectedExtension, ExtensionSession, ExtensionSessionState, Loaded, MepTransport, Session,
    TransportError, TransportState,
};
use crate::{DaemonError, Result};
use async_trait::async_trait;
use morphir_extension_sdk::ExtensionType;
use serde::{Serialize, de::DeserializeOwned};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::task::JoinHandle;
use tokio::time::timeout;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_STDERR_BYTES: usize = 256 * 1024;

/// A native extension command with an explicit identity and working directory.
#[derive(Debug, Clone)]
pub struct ProcessLaunch {
    extension_id: String,
    program: PathBuf,
    args: Vec<OsString>,
    working_directory: PathBuf,
    environment: Vec<(OsString, OsString)>,
    request_timeout: Duration,
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
            program: program.into(),
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
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
    Ready(Box<InitializeResult>),
    Stopped,
}

/// A MEP session carried over a child process's standard streams.
pub struct SpawnedProcessSession {
    expected_extension_id: String,
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr_task: Option<JoinHandle<std::io::Result<Vec<u8>>>>,
    stderr_output: String,
    next_request_id: u64,
    request_timeout: Duration,
    state: ProcessSessionData,
}

impl SpawnedProcessSession {
    /// Start a native extension and connect its standard streams.
    pub async fn spawn(launch: ProcessLaunch) -> Result<Self> {
        validate_launch(&launch)?;

        let mut command = Command::new(&launch.program);
        command
            .args(&launch.args)
            .current_dir(&launch.working_directory)
            .env_clear()
            .envs(launch.environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = command.spawn().map_err(|error| {
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
            child,
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
            stderr_task: Some(stderr_task),
            stderr_output: String::new(),
            next_request_id: 1,
            request_timeout: launch.request_timeout,
            state: ProcessSessionData::Starting,
        })
    }

    /// Start a native extension behind the shared typestate session controller.
    pub async fn spawn_typestate(
        launch: ProcessLaunch,
    ) -> Result<Session<SpawnedProcessSession, Loaded>> {
        Ok(Session::loaded(Self::spawn(launch).await?))
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

    fn ready_session(&self) -> Result<&InitializeResult> {
        match &self.state {
            ProcessSessionData::Ready(initialized) => Ok(initialized),
            ProcessSessionData::Starting | ProcessSessionData::Stopped => Err(
                DaemonError::Extension("Extension session is not ready".to_string()),
            ),
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
        self.collect_stderr().await?;
        self.state = ProcessSessionData::Stopped;
        Ok(())
    }
}

#[async_trait]
impl MepTransport for SpawnedProcessSession {
    fn expected_extension(&self) -> ExpectedExtension {
        ExpectedExtension::identified(self.expected_extension_id.clone())
    }

    async fn exchange(
        &mut self,
        request: ExtensionRequest,
    ) -> std::result::Result<ExtensionResponse, TransportError> {
        let method = request.method.clone();
        let exchange = async {
            let stdin = self.stdin.as_mut().ok_or_else(|| {
                DaemonError::Extension("Extension process stdin is closed".to_string())
            })?;
            write_frame(stdin, &request).await?;
            let frame = read_frame(&mut self.stdout).await?;
            serde_json::from_slice::<ExtensionResponse>(&frame).map_err(DaemonError::from)
        };
        let result = match timeout(self.request_timeout, exchange).await {
            Ok(result) => result,
            Err(_) => Err(DaemonError::Extension(format!(
                "Extension request '{}' timed out after {:?}",
                method, self.request_timeout
            ))),
        };
        match result {
            Ok(response) => Ok(response),
            Err(error) => Err(match self.abort_process().await {
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

    async fn terminate(&mut self) -> std::result::Result<TransportState, TransportError> {
        self.stdin.take();
        let status = match timeout(self.request_timeout, self.child.wait()).await {
            Ok(status) => status.map_err(|error| {
                TransportError::new(error.into(), TransportState::Indeterminate)
            })?,
            Err(_) => {
                let error = DaemonError::Extension(format!(
                    "Extension process did not exit after {:?}",
                    self.request_timeout
                ));
                return Err(match self.abort_process().await {
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
        self.collect_stderr()
            .await
            .map_err(|error| TransportError::new(error, TransportState::Stopped))?;
        self.state = ProcessSessionData::Stopped;
        if !status.success() {
            return Err(TransportError::new(
                DaemonError::Extension(format!("Extension process exited with status {status}")),
                TransportState::Stopped,
            ));
        }
        Ok(TransportState::Stopped)
    }
}

impl<S> Session<SpawnedProcessSession, S> {
    /// Report whether the child process is still running without exposing transport I/O.
    pub fn process_is_running(&mut self) -> Result<bool> {
        self.transport_mut_internal().is_running()
    }

    /// Return captured child-process diagnostics without exposing transport I/O.
    pub fn process_stderr_output(&self) -> &str {
        self.transport_internal().stderr_output()
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
        let validation = if !offered_versions.contains(&initialized.protocol_version) {
            Err(DaemonError::Extension(format!(
                "Extension selected protocol version '{}' that the host did not offer",
                initialized.protocol_version
            )))
        } else if initialized.extension.id != self.expected_extension_id {
            Err(DaemonError::Extension(format!(
                "Extension identity changed during initialization: expected '{}', initialized '{}'",
                self.expected_extension_id, initialized.extension.id
            )))
        } else {
            Ok(())
        };
        if let Err(error) = validation {
            return Err(self.abort_with_error(error).await);
        }

        self.state = ProcessSessionData::Ready(Box::new(initialized.clone()));
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

        self.call(method, params).await
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.ready_session()?;
        let _: serde_json::Value = self.call(methods::SHUTDOWN, serde_json::json!({})).await?;
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
    if !Path::new(&launch.program).is_file() {
        return Err(DaemonError::Extension(format!(
            "Extension executable does not exist: {}",
            launch.program.display()
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

fn required_capability(method: &str) -> Option<ExtensionType> {
    match method {
        methods::COMPILE => Some(ExtensionType::Frontend),
        methods::GENERATE => Some(ExtensionType::Backend),
        methods::VALIDATE => Some(ExtensionType::Validator),
        methods::TRANSFORM => Some(ExtensionType::Transform),
        _ => None,
    }
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
    use tokio::io::{BufReader, duplex};

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

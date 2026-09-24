//! A running extension process and its standard streams.

use crate::process::frame::{read_frame, write_frame};
use crate::process::launch::{ProcessLaunch, ProcessProgram};
use crate::process::stage::prepare_program;
use morphir_extension_sdk::protocol::ExtensionRequest;
use morphir_host::{ChannelCause, ChannelState, HostError};
use serde::Serialize;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::task::JoinHandle;
use tokio::time::timeout;

const MAX_STDERR_BYTES: usize = 256 * 1024;
const EXECUTABLE_BUSY_RETRIES: usize = 4;
const EXECUTABLE_BUSY_RETRY_DELAY: Duration = Duration::from_millis(10);

/// A spawned extension process, its standard streams, and its diagnostics.
///
/// The child starts with an empty environment plus the launch's variables,
/// and is killed when this value is dropped. Standard error is read in the
/// background and only the last 256 KiB are kept. A staged executable's
/// directory lives as long as this value.
///
/// Every read, write and wait runs under the launch's request timeout. A
/// timeout returns [`HostError::Channel`] with [`ChannelState::Indeterminate`]
/// and does not stop the child: the caller decides whether to abort it. No
/// other failure uses that variant, so callers can give a timeout their own
/// text.
pub struct ProcessChild {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr_task: Option<JoinHandle<std::io::Result<Vec<u8>>>>,
    stderr_output: String,
    request_timeout: Duration,
    _staged_program: Option<tempfile::TempDir>,
}

impl ProcessChild {
    /// Check the launch, stage its executable if needed, and start it.
    pub async fn spawn(launch: &ProcessLaunch) -> Result<Self, HostError> {
        validate_launch(launch)?;

        let (program, staged_program) = prepare_program(launch.program()).await?;
        let mut command = Command::new(&program);
        command
            .args(launch.args())
            .current_dir(launch.working_directory())
            .env_clear()
            .envs(launch.environment().iter().cloned())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = spawn_child(&mut command).await.map_err(|error| {
            HostError::Invalid(format!(
                "Failed to start extension '{}': {}",
                launch.extension_id(),
                error
            ))
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            HostError::Invalid("Extension process stdin was not captured".to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            HostError::Invalid("Extension process stdout was not captured".to_string())
        })?;
        let mut stderr = child.stderr.take().ok_or_else(|| {
            HostError::Invalid("Extension process stderr was not captured".to_string())
        })?;
        let stderr_task = tokio::spawn(async move { read_bounded_tail(&mut stderr).await });

        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
            stderr_task: Some(stderr_task),
            stderr_output: String::new(),
            request_timeout: launch.configured_request_timeout(),
            _staged_program: staged_program,
        })
    }

    /// The timeout applied to each read, write and wait.
    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    /// Write one frame to the child's standard input, under the full request
    /// timeout.
    pub async fn write<T: Serialize>(&mut self, message: &T) -> Result<(), HostError> {
        let request_timeout = self.request_timeout;
        self.write_within(message, request_timeout).await
    }

    /// Read one frame body from the child's standard output, under the full
    /// request timeout.
    pub async fn read(&mut self) -> Result<Vec<u8>, HostError> {
        let request_timeout = self.request_timeout;
        self.read_within(request_timeout).await
    }

    /// Write one frame to the child's standard input, under `duration`
    /// rather than the full request timeout.
    ///
    /// The timeout text always names the full configured request timeout, so
    /// a caller that shares one timeout budget across a write and the read
    /// that follows it (see [`Self::read_within`]) reports the same duration
    /// a caller using [`Self::write`] would.
    ///
    /// Used by `ProcessChannel` to keep one exchange under one timeout.
    #[doc(hidden)]
    pub async fn write_within<T: Serialize>(
        &mut self,
        message: &T,
        duration: Duration,
    ) -> Result<(), HostError> {
        let request_timeout = self.request_timeout;
        match timeout(duration, self.write_now(message)).await {
            Ok(result) => result,
            Err(_) => Err(timed_out(format!(
                "Extension process write timed out after {request_timeout:?}"
            ))),
        }
    }

    /// Read one frame body from the child's standard output, under
    /// `duration` rather than the full request timeout.
    ///
    /// A `duration` of zero times out at once, without attempting a read:
    /// a duration this short only ever comes from a budget an earlier write
    /// already spent, so there is nothing left to wait for. The timeout text
    /// always names the full configured request timeout, matching
    /// [`Self::read`].
    ///
    /// Used by `ProcessChannel` to keep one exchange under one timeout.
    #[doc(hidden)]
    pub async fn read_within(&mut self, duration: Duration) -> Result<Vec<u8>, HostError> {
        let request_timeout = self.request_timeout;
        if duration.is_zero() {
            return Err(timed_out(format!(
                "Extension process read timed out after {request_timeout:?}"
            )));
        }
        match timeout(duration, read_frame(&mut self.stdout)).await {
            Ok(result) => result,
            Err(_) => Err(timed_out(format!(
                "Extension process read timed out after {request_timeout:?}"
            ))),
        }
    }

    /// Write one request and read one frame body, under a single timeout.
    ///
    /// The timeout text names the request's method.
    ///
    /// Used by the daemon's compatibility session.
    #[doc(hidden)]
    pub async fn exchange(&mut self, request: &ExtensionRequest) -> Result<Vec<u8>, HostError> {
        let request_timeout = self.request_timeout;
        let exchange = async {
            self.write_now(request).await?;
            read_frame(&mut self.stdout).await
        };
        match timeout(request_timeout, exchange).await {
            Ok(result) => result,
            Err(_) => Err(timed_out(format!(
                "Extension request '{}' timed out after {:?}",
                request.method, request_timeout
            ))),
        }
    }

    /// Stop the child at once: close stdin, kill it if it runs, wait for it,
    /// and cancel the standard error reader.
    pub async fn abort(&mut self) -> Result<(), HostError> {
        self.stdin.take();
        if self.child.try_wait()?.is_none() {
            self.child.kill().await?;
        }
        let _ = self.child.wait().await?;
        if let Some(stderr_task) = self.stderr_task.take() {
            stderr_task.abort();
        }
        Ok(())
    }

    /// Close stdin, wait for the child to exit, and collect standard error.
    ///
    /// Does not stop a child that outlives the timeout.
    pub async fn wait_for_exit(&mut self) -> Result<ExitStatus, HostError> {
        let status = self.wait_for_status().await?;
        self.collect_stderr().await?;
        Ok(status)
    }

    /// Close stdin and wait for the child to exit, without collecting
    /// standard error.
    ///
    /// Part of [`Self::wait_for_exit`], for callers that treat a failed wait
    /// and a failed standard error read differently.
    ///
    /// Used by the daemon's compatibility session.
    #[doc(hidden)]
    pub async fn wait_for_status(&mut self) -> Result<ExitStatus, HostError> {
        self.stdin.take();
        let request_timeout = self.request_timeout;
        match timeout(request_timeout, self.child.wait()).await {
            Ok(status) => Ok(status?),
            Err(_) => Err(timed_out(format!(
                "Extension process did not exit after {request_timeout:?}"
            ))),
        }
    }

    /// Wait for the standard error reader and keep what it read.
    ///
    /// Part of [`Self::wait_for_exit`]. If the reader does not finish within
    /// the timeout, it is cancelled and nothing is kept.
    ///
    /// Used by the daemon's compatibility session.
    #[doc(hidden)]
    pub async fn collect_stderr(&mut self) -> Result<(), HostError> {
        let Some(mut stderr_task) = self.stderr_task.take() else {
            return Ok(());
        };
        match timeout(self.request_timeout, &mut stderr_task).await {
            Ok(result) => {
                let output = result.map_err(|error| {
                    HostError::Invalid(format!("Failed to join extension stderr reader: {error}"))
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

    /// Standard error collected after the child exited.
    pub fn stderr_output(&self) -> &str {
        &self.stderr_output
    }

    /// Report whether the child is still running.
    pub fn is_running(&mut self) -> Result<bool, HostError> {
        Ok(self.child.try_wait()?.is_none())
    }

    /// Report whether standard output has no unread bytes left.
    ///
    /// A conforming guest writes only the frames the host reads, so leftover
    /// bytes after exit are output that was not framed as a response.
    ///
    /// Used by the daemon's compatibility session.
    #[doc(hidden)]
    pub async fn stdout_is_exhausted(&mut self) -> Result<bool, HostError> {
        let request_timeout = self.request_timeout;
        match timeout(request_timeout, self.stdout.fill_buf()).await {
            Ok(result) => Ok(result?.is_empty()),
            Err(_) => Err(HostError::Invalid(format!(
                "Timed out while checking extension stdout after {request_timeout:?}"
            ))),
        }
    }

    async fn write_now<T: Serialize>(&mut self, message: &T) -> Result<(), HostError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| HostError::Invalid("Extension process stdin is closed".to_string()))?;
        write_frame(stdin, message).await
    }
}

fn timed_out(message: String) -> HostError {
    HostError::Channel {
        message,
        state: ChannelState::Indeterminate,
        cause: ChannelCause::Transport,
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

fn validate_launch(launch: &ProcessLaunch) -> Result<(), HostError> {
    if launch.extension_id().trim().is_empty() {
        return Err(HostError::Invalid(
            "Extension process identity cannot be empty".to_string(),
        ));
    }
    if let ProcessProgram::Path(program) = launch.program()
        && !program.is_file()
    {
        return Err(HostError::Invalid(format!(
            "Extension executable does not exist: {}",
            program.display()
        )));
    }
    if !launch.working_directory().is_dir() {
        return Err(HostError::Invalid(format!(
            "Extension working directory does not exist: {}",
            launch.working_directory().display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stderr_capture_retains_only_the_bounded_tail() {
        let mut output = b"old diagnostics".to_vec();
        append_bounded_tail(&mut output, b"new diagnostics", 16);

        assert_eq!(output, b"snew diagnostics");

        append_bounded_tail(&mut output, b"0123456789abcdefghijkl", 16);
        assert_eq!(output, b"6789abcdefghijkl");
    }
}

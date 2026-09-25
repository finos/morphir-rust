//! A `morphir_host::Channel` over a child process's standard streams.

use crate::process::child::ProcessChild;
use crate::process::launch::ProcessLaunch;
use async_trait::async_trait;
use morphir_extension_sdk::protocol::ExtensionResponse;
use morphir_host::{
    Channel, ChannelCause, ChannelError, ChannelState, ExpectedExtension, HostError, Outgoing,
};
use std::time::Instant;

/// A guest that runs as a child process and speaks MEP over stdio.
///
/// `close` does not send `morphir.exit`: the connection sends it through
/// `send` first. When `send`, `receive` or `close` fails, the channel kills
/// the child before it returns the error. The error's state is `Stopped`
/// when the kill succeeded and `Indeterminate` when it failed too.
///
/// One request's write and the response `receive` reads for it share a
/// single request-timeout budget, rather than each getting a full timeout of
/// its own. `send` records a deadline before it writes a request; `receive`
/// waits only for what is left of that deadline, so a slow write leaves less
/// time for the read that follows. A `receive` with no preceding request (no
/// recorded deadline) still gets a full request timeout. Notifications, such
/// as `morphir.exit`, keep their own full timeout and never touch the
/// request deadline.
pub struct ProcessChannel {
    child: ProcessChild,
    expectation: ExpectedExtension,
    method: Option<String>,
    /// The deadline a request's write recorded, for the `receive` that reads
    /// its response. `None` once spent, cleared, or never set.
    deadline: Option<Instant>,
}

impl ProcessChannel {
    /// Start the process that `launch` describes.
    pub async fn spawn(launch: ProcessLaunch) -> Result<Self, HostError> {
        let child = ProcessChild::spawn(&launch).await?;
        Ok(Self {
            child,
            expectation: launch.expectation(),
            method: None,
            deadline: None,
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
        self.deadline = None;
        let cause = ChannelCause::of(&error);
        match self.child.abort().await {
            Ok(()) => ChannelError {
                message: error.to_string(),
                state: ChannelState::Stopped,
                cause,
            },
            Err(cleanup) => ChannelError {
                message: format!("{error}; process cleanup also failed: {cleanup}"),
                state: ChannelState::Indeterminate,
                cause,
            },
        }
    }

    /// Give a timeout the text of the request or notification it hit.
    fn name_timeout(&self, error: HostError, subject: impl FnOnce() -> String) -> HostError {
        match error {
            HostError::Channel { state, cause, .. } => HostError::Channel {
                message: format!(
                    "{} timed out after {:?}",
                    subject(),
                    self.child.request_timeout()
                ),
                state,
                cause,
            },
            other => other,
        }
    }

    /// Name the exchange a timeout interrupted.
    ///
    /// `receive` is only ever called after `send` writes a request, so the
    /// method is known by the time a real exchange times out. A `receive`
    /// with no prior `send` (host misuse, or a test) has no method to name,
    /// so it reports a plain response timeout instead of an empty method
    /// name.
    fn request_subject(&self) -> String {
        match &self.method {
            Some(method) => format!("Extension request '{method}'"),
            None => "Extension response".to_string(),
        }
    }
}

#[async_trait]
impl Channel for ProcessChannel {
    async fn send(&mut self, message: Outgoing) -> Result<(), ChannelError> {
        let result = match &message {
            Outgoing::Request(request) => {
                self.method = Some(request.method.clone());
                let request_timeout = self.child.request_timeout();
                self.deadline = Some(Instant::now() + request_timeout);
                self.child.write_within(request, request_timeout).await
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
        // Spend the deadline a preceding request's `send` recorded, so the
        // write and the read that follows it share one timeout budget. A
        // `receive` with no such deadline (no preceding request) keeps the
        // full request timeout.
        let duration = match self.deadline.take() {
            Some(deadline) => deadline.saturating_duration_since(Instant::now()),
            None => self.child.request_timeout(),
        };
        let response = match self.child.read_within(duration).await {
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
                cause: ChannelCause::Transport,
            }),
            Err(error) => Err(self.fail(error).await),
        }
    }

    async fn abort(&mut self) -> Result<ChannelState, ChannelError> {
        self.child
            .abort()
            .await
            .map(|()| ChannelState::Stopped)
            .map_err(|error| {
                let cause = ChannelCause::of(&error);
                ChannelError {
                    message: error.to_string(),
                    state: ChannelState::Indeterminate,
                    cause,
                }
            })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::process::launch::ProcessLaunch;
    use morphir_extension_sdk::protocol::{ExtensionRequest, methods};
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    /// A guest that never answers, so a `receive` against it always times out.
    const HANG: &str = "#!/bin/sh\nPATH=/usr/bin:/bin\nwhile true; do sleep 1; done\n";

    /// A guest that waits one second, then reads one request frame and
    /// answers it with a fixed success result for id 1.
    ///
    /// Used to prove a request's write and the `receive` that follows it
    /// share one timeout budget: this guest always answers well inside a
    /// full request timeout, so only a deadline a write already spent can
    /// make `receive` fail before the guest gets the chance to.
    const ANSWERS_AFTER_A_DELAY: &str = r#"#!/bin/sh
PATH=/usr/bin:/bin
sleep 1
len=0
while IFS= read -r header; do
  header=$(printf '%s' "$header" | tr -d '\r')
  case "$header" in
    Content-Length:*) len=$(printf '%s' "${header#Content-Length:}" | tr -d ' ');;
    '') dd bs=1 count="$len" >/dev/null 2>/dev/null
        out='{"jsonrpc":"2.0","result":{},"id":1}'
        printf 'Content-Length: %s\r\n\r\n%s' "${#out}" "$out"
        exit 0;;
  esac
done
"#;

    fn guest(dir: &tempfile::TempDir, script: &str) -> std::path::PathBuf {
        let path = dir.path().join("guest.sh");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    fn ping() -> Outgoing {
        Outgoing::Request(ExtensionRequest::new(methods::PING, serde_json::json!({}), 1).unwrap())
    }

    /// A guest that exits at once, before it reads or writes anything.
    const EXITS_AT_ONCE: &str = "#!/bin/sh\nexit 0\n";

    /// A guest that exits before the host is done talking to it can be
    /// caught two ways, depending on timing: the write can hit a pipe the
    /// guest already closed (an I/O error), or the write can land in the
    /// pipe's buffer before the guest closes it, so only the `receive` that
    /// follows sees the closed stdout. Either way the error must carry the
    /// cause that matches its own message.
    #[tokio::test]
    async fn a_guest_that_exits_at_once_fails_with_the_cause_its_message_matches() {
        let dir = tempfile::tempdir().unwrap();
        let launch = ProcessLaunch::new("guest", guest(&dir, EXITS_AT_ONCE), dir.path())
            .request_timeout(Duration::from_millis(500));
        let mut channel = ProcessChannel::spawn(launch).await.unwrap();
        // Give the guest a chance to exit before the host writes to it.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let error = match channel.send(ping()).await {
            Err(error) => error,
            Ok(()) => channel.receive().await.unwrap_err(),
        };

        match error.cause {
            ChannelCause::Io => assert!(
                error.message.starts_with("IO error: "),
                "an IO cause should carry an IO error message: {}",
                error.message
            ),
            ChannelCause::Transport => assert!(
                error
                    .message
                    .contains("closed stdout before a response frame"),
                "a transport cause here should be the closed-stdout race: {}",
                error.message
            ),
            other => panic!("unexpected {other:?} cause: {}", error.message),
        }
    }

    #[tokio::test]
    async fn a_receive_before_any_send_reports_a_response_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let launch = ProcessLaunch::new("guest", guest(&dir, HANG), dir.path())
            .request_timeout(Duration::from_millis(300));
        let mut channel = ProcessChannel::spawn(launch).await.unwrap();

        let error = channel.receive().await.unwrap_err();

        assert_eq!(error.message, "Extension response timed out after 300ms");
    }

    /// `receive` must spend what is left of the deadline `send` recorded for
    /// its request, not a fresh full timeout.
    ///
    /// A real guest process backs this test: it would answer about a second
    /// from now, well inside the full 2-second request timeout. Making the
    /// write itself measurably slow is not deterministic (it depends on the
    /// OS pipe buffer filling up), so this test instead does what a write
    /// that had already spent the whole budget would leave behind: a
    /// deadline that has already passed. If `receive` used a fresh timeout
    /// instead of the recorded deadline, it would wait for the guest and
    /// succeed about a second later; sharing the budget, it must fail at
    /// once instead.
    #[tokio::test]
    async fn receive_spends_the_deadline_a_slow_write_would_have_left_it() {
        let dir = tempfile::tempdir().unwrap();
        let launch = ProcessLaunch::new("guest", guest(&dir, ANSWERS_AFTER_A_DELAY), dir.path())
            .request_timeout(Duration::from_secs(2));
        let mut channel = ProcessChannel::spawn(launch).await.unwrap();

        channel.send(ping()).await.unwrap();
        // Stand in for a write that had already spent the whole 2-second
        // budget: the deadline `send` just recorded is overwritten with one
        // already in the past.
        channel.deadline = Some(Instant::now() - Duration::from_millis(1));

        let started = Instant::now();
        let error = channel.receive().await.unwrap_err();
        let elapsed = started.elapsed();

        assert_eq!(error.state, ChannelState::Stopped);
        assert_eq!(
            error.message,
            "Extension request 'morphir.ping' timed out after 2s"
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "receive should fail at once on an already-spent deadline, took {elapsed:?}"
        );
    }

    /// A `receive` with no recorded deadline still gets the full request
    /// timeout, not an immediate timeout from a stray `None` deadline.
    #[tokio::test]
    async fn receive_with_no_recorded_deadline_still_gets_the_full_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let launch = ProcessLaunch::new("guest", guest(&dir, ANSWERS_AFTER_A_DELAY), dir.path())
            .request_timeout(Duration::from_secs(2));
        let mut channel = ProcessChannel::spawn(launch).await.unwrap();

        channel.send(ping()).await.unwrap();
        // Stand in for "no preceding request": clear the deadline `send`
        // just recorded, the same state `receive` sees before any `send`.
        channel.deadline = None;

        // The guest answers about a second from now. With no deadline to
        // spend, `receive` must fall back to the full 2-second timeout and
        // wait for it, rather than timing out at once.
        let response = channel.receive().await.unwrap();

        assert_eq!(response.id, 1);
    }
}

#![cfg(unix)]

use morphir_extension_sdk::protocol::{ExtensionNotification, ExtensionRequest, methods};
use morphir_host::{Channel, ChannelState, Outgoing};
use morphir_host_native::process::{ProcessChannel, ProcessLaunch};
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

/// A guest that answers every request frame with a success result for id 1,
/// counts notification frames (frames with no `"id"`), and exits 0 when stdin
/// closes after it writes `notifications=<n>` to stderr.
///
/// The script uses only POSIX `sh`: `read` takes one header line, `tr`
/// removes the carriage return, and `dd bs=1` reads exactly the body bytes.
/// The launch clears the environment, so the script sets its own `PATH`.
const ECHO: &str = r#"#!/bin/sh
PATH=/usr/bin:/bin
len=0
notifications=0
while IFS= read -r header; do
  header=$(printf '%s' "$header" | tr -d '\r')
  case "$header" in
    Content-Length:*) len=$(printf '%s' "${header#Content-Length:}" | tr -d ' ');;
    '') body=$(dd bs=1 count="$len" 2>/dev/null)
        case "$body" in
          *'"id"'*) out='{"jsonrpc":"2.0","result":{},"id":1}'
                    printf 'Content-Length: %s\r\n\r\n%s' "${#out}" "$out";;
          *) notifications=$((notifications + 1));;
        esac;;
  esac
done
printf 'notifications=%s\n' "$notifications" >&2
exit 0
"#;

/// A guest that never exits on its own.
const HANG: &str = "#!/bin/sh\nPATH=/usr/bin:/bin\nwhile true; do sleep 1; done\n";

fn guest(dir: &tempfile::TempDir, script: &str) -> std::path::PathBuf {
    let path = dir.path().join("guest.sh");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

fn ping() -> Outgoing {
    Outgoing::Request(ExtensionRequest::new(methods::PING, serde_json::json!({}), 1).unwrap())
}

#[tokio::test]
async fn close_after_exit_waits_without_a_second_exit() {
    let dir = tempfile::tempdir().unwrap();
    let launch = ProcessLaunch::new("guest", guest(&dir, ECHO), dir.path())
        .request_timeout(Duration::from_secs(5));
    let mut channel = ProcessChannel::spawn(launch).await.unwrap();
    channel.send(ping()).await.unwrap();
    let response = channel.receive().await.unwrap();
    assert_eq!(response.id, 1);
    channel
        .send(Outgoing::Notification(
            ExtensionNotification::without_params(methods::EXIT),
        ))
        .await
        .unwrap();
    assert_eq!(channel.close().await.unwrap(), ChannelState::Stopped);
    assert_eq!(channel.stderr_output().trim(), "notifications=1");
}

#[tokio::test]
async fn abort_kills_the_child_without_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let launch = ProcessLaunch::new("guest", guest(&dir, HANG), dir.path())
        .request_timeout(Duration::from_secs(30));
    let mut channel = ProcessChannel::spawn(launch).await.unwrap();
    let started = Instant::now();
    assert_eq!(channel.abort().await.unwrap(), ChannelState::Stopped);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_receive_timeout_kills_the_child_and_reports_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let launch = ProcessLaunch::new("guest", guest(&dir, HANG), dir.path())
        .request_timeout(Duration::from_millis(300));
    let mut channel = ProcessChannel::spawn(launch).await.unwrap();
    channel.send(ping()).await.unwrap();
    let error = channel.receive().await.unwrap_err();
    assert_eq!(error.state, ChannelState::Stopped);
    assert_eq!(
        error.message,
        "Extension request 'morphir.ping' timed out after 300ms"
    );
}

#[tokio::test]
async fn close_reports_a_non_zero_exit_status_as_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let launch = ProcessLaunch::new("guest", guest(&dir, "#!/bin/sh\nexit 3\n"), dir.path())
        .request_timeout(Duration::from_secs(5));
    let mut channel = ProcessChannel::spawn(launch).await.unwrap();
    let error = channel.close().await.unwrap_err();
    assert_eq!(error.state, ChannelState::Stopped);
    assert!(
        error
            .message
            .starts_with("Extension process exited with status"),
        "{}",
        error.message
    );
}

#[tokio::test]
async fn close_aborts_a_guest_that_does_not_exit() {
    let dir = tempfile::tempdir().unwrap();
    let launch = ProcessLaunch::new("guest", guest(&dir, HANG), dir.path())
        .request_timeout(Duration::from_millis(300));
    let mut channel = ProcessChannel::spawn(launch).await.unwrap();
    let error = channel.close().await.unwrap_err();
    assert_eq!(error.state, ChannelState::Stopped);
    assert_eq!(error.message, "Extension process did not exit after 300ms");
}

#[tokio::test]
async fn stderr_is_collected_after_the_guest_exits() {
    let dir = tempfile::tempdir().unwrap();
    let script = "#!/bin/sh\nprintf 'guest diagnostics' >&2\nexit 0\n";
    let launch = ProcessLaunch::new("guest", guest(&dir, script), dir.path())
        .request_timeout(Duration::from_secs(5));
    let mut channel = ProcessChannel::spawn(launch).await.unwrap();
    assert_eq!(channel.close().await.unwrap(), ChannelState::Stopped);
    assert_eq!(channel.stderr_output(), "guest diagnostics");
}

#[tokio::test]
async fn a_missing_executable_is_refused_before_spawning() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");
    let error = ProcessChannel::spawn(ProcessLaunch::new("guest", &missing, dir.path()))
        .await
        .err()
        .expect("a missing executable must be refused");
    assert_eq!(
        error.to_string(),
        format!("Extension executable does not exist: {}", missing.display())
    );
}

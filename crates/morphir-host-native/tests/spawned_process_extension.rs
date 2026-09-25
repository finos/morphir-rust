//! Conformance tests for a real native extension child process.
//!
//! Build the fixture and provide its path before running this ignored test:
//!
//! `cargo build -p morphir-daemon --example mep-native-backend`
//! `MEP_NATIVE_FIXTURE=target/debug/examples/mep-native-backend cargo test -p morphir-host-native --test spawned_process_extension -- --ignored`

mod support;

use morphir_extension_sdk::{GenerateRequest, protocol::methods};
use morphir_host::{CallError, ChannelState, Session};
use morphir_host_native::process::ProcessLaunch;
use serde_json::json;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use support::mep::{SharedProcess, backend_conformance, channel_state, host_config};

fn native_fixture_path() -> PathBuf {
    let path = std::env::var_os("MEP_NATIVE_FIXTURE")
        .map(PathBuf::from)
        .expect("MEP_NATIVE_FIXTURE should point at the independently built native extension");
    if path.is_absolute() {
        path
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }
}

fn native_launch() -> ProcessLaunch {
    ProcessLaunch::new(
        "mep-native-backend",
        native_fixture_path(),
        std::env::current_dir().expect("the test working directory should exist"),
    )
}

fn a_distribution_with_one_value() -> serde_json::Value {
    json!({
        "name": "conformance",
        "modules": [{
            "name": "main",
            "values": [{
                "name": "answer",
                "body": {
                    "kind": "literal",
                    "value": { "type": "int", "value": 42 }
                }
            }]
        }]
    })
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn completes_mep_through_a_real_child_process() {
    let process = SharedProcess::spawn(native_launch()).await;

    backend_conformance(
        process.connection().await,
        "json",
        a_distribution_with_one_value(),
        json!("not Morphir IR"),
    )
    .await;

    assert!(!process.is_running().await);
    assert!(
        process
            .stderr_output()
            .await
            .contains("native MEP fixture started")
    );
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn compatibility_session_rejects_exit_as_a_request() {
    let process = SharedProcess::spawn(native_launch()).await;
    let mut session = Session::open(
        process.connection().await,
        &host_config("exit-request-test", "0.1.0"),
    )
    .await
    .expect("the fixture should initialize");

    let error = session
        .call::<_, serde_json::Value>(methods::EXIT, json!({}))
        .await
        .expect_err("exit must use a notification during shutdown");

    assert!(
        matches!(error, CallError::Rejected(_)),
        "a lifecycle method is refused and the session stays ready: {error:?}"
    );
    assert!(error.to_string().contains("lifecycle method"));
    session
        .close()
        .await
        .expect("the ready session should still shut down cleanly");
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn kills_a_child_that_exceeds_the_request_timeout() {
    let launch = native_launch()
        .env("MEP_FIXTURE_HANG_GENERATE", "1")
        .request_timeout(Duration::from_millis(100));
    let process = SharedProcess::spawn(launch).await;
    let mut session = Session::open(
        process.connection().await,
        &host_config("timeout-test", "0.1.0"),
    )
    .await
    .unwrap_or_else(|error| panic!("initialization failed: {error}"));

    let error = match session
        .call::<_, serde_json::Value>(
            methods::GENERATE,
            GenerateRequest {
                ir: a_distribution_with_one_value(),
                target: "json".into(),
                options: Default::default(),
            },
        )
        .await
    {
        Err(CallError::Failed(error)) => error,
        Ok(_) => panic!("the hung request should time out"),
        Err(error) => panic!("the request did not fail the session: {error:?}"),
    };

    assert!(error.to_string().contains("timed out"));
    assert_eq!(
        channel_state(&error),
        ChannelState::Stopped,
        "the killed child should be stopped"
    );
    assert!(!process.is_running().await);
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn kills_a_child_that_does_not_exit_after_shutdown() {
    let launch = native_launch()
        .env("MEP_FIXTURE_IGNORE_SHUTDOWN", "1")
        .request_timeout(Duration::from_millis(100));
    let process = SharedProcess::spawn(launch).await;
    let session = Session::open(
        process.connection().await,
        &host_config("shutdown-timeout-test", "0.1.0"),
    )
    .await
    .unwrap_or_else(|error| panic!("initialization failed: {error}"));

    let error = session
        .close()
        .await
        .expect_err("the child should exceed the shutdown grace period");

    assert!(error.to_string().contains("did not exit"));
    assert_eq!(
        channel_state(&error),
        ChannelState::Stopped,
        "the killed child should be stopped"
    );
    assert!(!process.is_running().await);
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn kills_a_child_after_failed_protocol_negotiation() {
    let launch = native_launch()
        .env("MEP_FIXTURE_UNSUPPORTED_PROTOCOL", "1")
        .request_timeout(Duration::from_millis(100));
    let process = SharedProcess::spawn(launch).await;

    let error = match Session::open(
        process.connection().await,
        &host_config("negotiation-test", "0.1.0"),
    )
    .await
    {
        Err(error) => error,
        Ok(_) => panic!("the extension should not select an unsupported protocol"),
    };

    assert!(error.to_string().contains("did not offer"));
    assert!(
        !error.to_string().contains("abort also failed"),
        "the child should be killed: {error}"
    );
    assert!(!process.is_running().await);
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn aborts_promptly_after_failed_protocol_negotiation() {
    let request_timeout = Duration::from_secs(5);
    let launch = native_launch()
        .env("MEP_FIXTURE_UNSUPPORTED_PROTOCOL", "1")
        .env("MEP_FIXTURE_HANG_AFTER_INITIALIZE", "1")
        .env("MEP_FIXTURE_HOLD_STDERR_OPEN", "1")
        .request_timeout(request_timeout);
    let process = SharedProcess::spawn(launch).await;
    let connection = process.connection().await;
    let config = host_config("prompt-abort-test", "0.1.0");

    let started = Instant::now();
    let error =
        match tokio::time::timeout(Duration::from_secs(1), Session::open(connection, &config))
            .await
            .expect("failed negotiation cleanup should not use the request timeout")
        {
            Err(error) => error,
            Ok(_) => panic!("the extension should not select an unsupported protocol"),
        };

    assert!(started.elapsed() < request_timeout);
    assert!(error.to_string().contains("did not offer"));
    assert!(
        !error.to_string().contains("abort also failed"),
        "the child should be killed: {error}"
    );
    assert!(!process.is_running().await);
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn compatibility_session_kills_a_child_after_a_malformed_envelope() {
    let launch = native_launch()
        .env("MEP_FIXTURE_INVALID_ENVELOPE", "1")
        .request_timeout(Duration::from_millis(100));
    let process = SharedProcess::spawn(launch).await;

    let error = match Session::open(
        process.connection().await,
        &host_config("invalid-envelope-test", "0.1.0"),
    )
    .await
    {
        Err(error) => error,
        Ok(_) => panic!("the malformed envelope should fail closed"),
    };

    assert!(error.to_string().contains("JSON-RPC version"));
    assert!(!process.is_running().await);
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn shutdown_does_not_wait_for_a_descendant_holding_stderr_open() {
    let launch = native_launch()
        .env("MEP_FIXTURE_HOLD_STDERR_OPEN", "1")
        .request_timeout(Duration::from_millis(100));
    let process = SharedProcess::spawn(launch).await;

    backend_conformance(
        process.connection().await,
        "json",
        a_distribution_with_one_value(),
        json!("not Morphir IR"),
    )
    .await;

    assert!(!process.is_running().await);
}

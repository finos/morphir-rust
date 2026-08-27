//! Conformance tests for a real native extension child process.
//!
//! Build the fixture and provide its path before running this ignored test:
//!
//! `cargo build -p morphir-daemon --example mep-native-backend`
//! `MEP_NATIVE_FIXTURE=target/debug/examples/mep-native-backend cargo test -p morphir-daemon --test spawned_process_extension -- --ignored`

mod support;

use morphir_daemon::extensions::{
    FailedSession, InvokeOutcome, ProcessLaunch, SpawnedProcessSession,
};
use morphir_extension_sdk::{
    GenerateRequest,
    protocol::{InitializeParams, PeerInfo, methods},
};
use serde_json::json;
use std::path::PathBuf;
use std::time::{Duration, Instant};

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
    let launch = ProcessLaunch::new(
        "mep-native-backend",
        native_fixture_path(),
        std::env::current_dir().expect("the test working directory should exist"),
    );
    let session = SpawnedProcessSession::spawn_typestate(launch)
        .await
        .expect("the host should start the native extension fixture");

    let mut session = support::mep::assert_backend_typestate_conformance(
        session,
        a_distribution_with_one_value(),
        json!("not Morphir IR"),
    )
    .await;

    assert!(
        !session
            .process_is_running()
            .expect("process status should be readable")
    );
    assert!(
        session
            .process_stderr_output()
            .contains("native MEP fixture started")
    );
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn kills_a_child_that_exceeds_the_request_timeout() {
    let launch = ProcessLaunch::new(
        "mep-native-backend",
        native_fixture_path(),
        std::env::current_dir().expect("the test working directory should exist"),
    )
    .env("MEP_FIXTURE_HANG_GENERATE", "1")
    .request_timeout(Duration::from_millis(100));
    let session = SpawnedProcessSession::spawn_typestate(launch)
        .await
        .expect("the host should start the native extension fixture");
    let session = session
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "timeout-test".into(),
                version: "0.1.0".into(),
            },
        })
        .await
        .unwrap_or_else(|failure| panic!("initialization failed: {}", failure.error()));

    let failure = match session
        .invoke::<serde_json::Value>(
            methods::GENERATE,
            serde_json::to_value(GenerateRequest {
                ir: a_distribution_with_one_value(),
                options: Default::default(),
            })
            .expect("the generation request should serialize"),
        )
        .await
    {
        InvokeOutcome::Failed(failure) => failure,
        InvokeOutcome::Success(_, _) => panic!("the hung request should time out"),
        InvokeOutcome::Rejected(_, error) => panic!("the request was rejected: {error}"),
    };

    assert!(failure.error().to_string().contains("timed out"));
    let mut session = match failure {
        FailedSession::Stopped(session, _) => session,
        FailedSession::Indeterminate(_, _) => panic!("the killed child should be stopped"),
    };
    assert!(
        !session
            .process_is_running()
            .expect("process status should be readable")
    );
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn kills_a_child_that_does_not_exit_after_shutdown() {
    let launch = ProcessLaunch::new(
        "mep-native-backend",
        native_fixture_path(),
        std::env::current_dir().expect("the test working directory should exist"),
    )
    .env("MEP_FIXTURE_IGNORE_SHUTDOWN", "1")
    .request_timeout(Duration::from_millis(100));
    let session = SpawnedProcessSession::spawn_typestate(launch)
        .await
        .expect("the host should start the native extension fixture");
    let session = session
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "shutdown-timeout-test".into(),
                version: "0.1.0".into(),
            },
        })
        .await
        .unwrap_or_else(|failure| panic!("initialization failed: {}", failure.error()));

    let failure = session
        .shutdown()
        .await
        .err()
        .expect("the child should exceed the shutdown grace period");

    assert!(failure.error().to_string().contains("did not exit"));
    let mut session = match failure {
        FailedSession::Stopped(session, _) => session,
        FailedSession::Indeterminate(_, _) => panic!("the killed child should be stopped"),
    };
    assert!(
        !session
            .process_is_running()
            .expect("process status should be readable")
    );
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn kills_a_child_after_failed_protocol_negotiation() {
    let launch = ProcessLaunch::new(
        "mep-native-backend",
        native_fixture_path(),
        std::env::current_dir().expect("the test working directory should exist"),
    )
    .env("MEP_FIXTURE_UNSUPPORTED_PROTOCOL", "1")
    .request_timeout(Duration::from_millis(100));
    let session = SpawnedProcessSession::spawn_typestate(launch)
        .await
        .expect("the host should start the native extension fixture");

    let failure = session
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "negotiation-test".into(),
                version: "0.1.0".into(),
            },
        })
        .await
        .err()
        .expect("the extension should not select an unsupported protocol");

    assert!(failure.error().to_string().contains("did not offer"));
    let mut session = match failure {
        FailedSession::Stopped(session, _) => session,
        FailedSession::Indeterminate(_, _) => panic!("the killed child should be stopped"),
    };
    assert!(
        !session
            .process_is_running()
            .expect("process status should be readable")
    );
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn aborts_promptly_after_failed_protocol_negotiation() {
    let request_timeout = Duration::from_secs(5);
    let launch = ProcessLaunch::new(
        "mep-native-backend",
        native_fixture_path(),
        std::env::current_dir().expect("the test working directory should exist"),
    )
    .env("MEP_FIXTURE_UNSUPPORTED_PROTOCOL", "1")
    .env("MEP_FIXTURE_HANG_AFTER_INITIALIZE", "1")
    .env("MEP_FIXTURE_HOLD_STDERR_OPEN", "1")
    .request_timeout(request_timeout);
    let session = SpawnedProcessSession::spawn_typestate(launch)
        .await
        .expect("the host should start the native extension fixture");

    let started = Instant::now();
    let failure = tokio::time::timeout(
        Duration::from_secs(1),
        session.initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "prompt-abort-test".into(),
                version: "0.1.0".into(),
            },
        }),
    )
    .await
    .expect("failed negotiation cleanup should not use the request timeout")
    .err()
    .expect("the extension should not select an unsupported protocol");

    assert!(started.elapsed() < request_timeout);
    assert!(failure.error().to_string().contains("did not offer"));
    let mut session = match failure {
        FailedSession::Stopped(session, _) => session,
        FailedSession::Indeterminate(_, _) => panic!("the killed child should be stopped"),
    };
    assert!(
        !session
            .process_is_running()
            .expect("process status should be readable")
    );
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn compatibility_session_kills_a_child_after_a_malformed_envelope() {
    use morphir_daemon::extensions::{ExtensionSession, ExtensionSessionState};

    let launch = ProcessLaunch::new(
        "mep-native-backend",
        native_fixture_path(),
        std::env::current_dir().expect("the test working directory should exist"),
    )
    .env("MEP_FIXTURE_INVALID_ENVELOPE", "1")
    .request_timeout(Duration::from_millis(100));
    let mut session = SpawnedProcessSession::spawn(launch)
        .await
        .expect("the host should start the native extension fixture");

    let error = session
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "invalid-envelope-test".into(),
                version: "0.1.0".into(),
            },
        })
        .await
        .expect_err("the malformed envelope should fail closed");

    assert!(error.to_string().contains("JSON-RPC version"));
    assert_eq!(session.state(), ExtensionSessionState::Stopped);
    assert!(
        !session
            .is_running()
            .expect("process status should be readable")
    );
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn shutdown_does_not_wait_for_a_descendant_holding_stderr_open() {
    let launch = ProcessLaunch::new(
        "mep-native-backend",
        native_fixture_path(),
        std::env::current_dir().expect("the test working directory should exist"),
    )
    .env("MEP_FIXTURE_HOLD_STDERR_OPEN", "1")
    .request_timeout(Duration::from_millis(100));
    let session = SpawnedProcessSession::spawn_typestate(launch)
        .await
        .expect("the host should start the native extension fixture");

    let mut session = support::mep::assert_backend_typestate_conformance(
        session,
        a_distribution_with_one_value(),
        json!("not Morphir IR"),
    )
    .await;

    assert!(
        !session
            .process_is_running()
            .expect("process status should be readable")
    );
}

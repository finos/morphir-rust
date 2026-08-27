//! Conformance tests for a real native extension child process.
//!
//! Build the fixture and provide its path before running this ignored test:
//!
//! `cargo build -p morphir-daemon --example mep-native-backend`
//! `MEP_NATIVE_FIXTURE=target/debug/examples/mep-native-backend cargo test -p morphir-daemon --test spawned_process_extension -- --ignored`

mod support;

use morphir_daemon::extensions::{ExtensionSession, ProcessLaunch, SpawnedProcessSession};
use morphir_extension_sdk::{
    GenerateRequest,
    protocol::{InitializeParams, PeerInfo, methods},
};
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;

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
    let session = SpawnedProcessSession::spawn(launch)
        .await
        .expect("the host should start the native extension fixture");

    let mut session = support::mep::assert_backend_session_conformance(
        session,
        a_distribution_with_one_value(),
        json!("not Morphir IR"),
    )
    .await;

    assert!(
        !session
            .is_running()
            .expect("process status should be readable")
    );
    assert!(
        session
            .stderr_output()
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
    let mut session = SpawnedProcessSession::spawn(launch)
        .await
        .expect("the host should start the native extension fixture");
    session
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "timeout-test".into(),
                version: "0.1.0".into(),
            },
        })
        .await
        .expect("the fixture should initialize before the timeout case");

    let error = session
        .invoke(
            methods::GENERATE,
            serde_json::to_value(GenerateRequest {
                ir: a_distribution_with_one_value(),
                options: Default::default(),
            })
            .expect("the generation request should serialize"),
        )
        .await
        .expect_err("the hung request should time out");

    assert!(error.to_string().contains("timed out"));
    assert!(
        !session
            .is_running()
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
    let mut session = SpawnedProcessSession::spawn(launch)
        .await
        .expect("the host should start the native extension fixture");
    session
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "shutdown-timeout-test".into(),
                version: "0.1.0".into(),
            },
        })
        .await
        .expect("the fixture should initialize before the shutdown timeout case");

    let error = session
        .shutdown()
        .await
        .expect_err("the child should exceed the shutdown grace period");

    assert!(error.to_string().contains("did not exit"));
    assert!(
        !session
            .is_running()
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
    let mut session = SpawnedProcessSession::spawn(launch)
        .await
        .expect("the host should start the native extension fixture");

    let error = session
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "negotiation-test".into(),
                version: "0.1.0".into(),
            },
        })
        .await
        .expect_err("the extension should not select an unsupported protocol");

    assert!(error.to_string().contains("did not offer"));
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
    let session = SpawnedProcessSession::spawn(launch)
        .await
        .expect("the host should start the native extension fixture");

    let mut session = support::mep::assert_backend_session_conformance(
        session,
        a_distribution_with_one_value(),
        json!("not Morphir IR"),
    )
    .await;

    assert!(
        !session
            .is_running()
            .expect("process status should be readable")
    );
}

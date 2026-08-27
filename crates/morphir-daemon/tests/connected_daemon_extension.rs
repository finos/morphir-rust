//! Conformance tests for a real extension daemon over JSON-RPC HTTP.
//!
//! Build the fixture and provide its path before running this ignored test:
//!
//! `cargo build -p morphir-daemon --example mep-http-backend`
//! `MEP_HTTP_FIXTURE=target/debug/examples/mep-http-backend cargo test -p morphir-daemon --test connected_daemon_extension -- --ignored`

mod support;

use morphir_daemon::extensions::{
    ConnectedDaemonSession, DaemonConnection, FailedSession, InvokeOutcome,
};
use morphir_extension_sdk::{
    GenerateRequest,
    protocol::{InitializeParams, PeerInfo, methods},
};
use serde_json::json;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

struct FixtureDaemon {
    child: Child,
    endpoint: String,
}

impl FixtureDaemon {
    async fn start(args: &[&str]) -> Self {
        let mut child = Command::new(http_fixture_path())
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .expect("the HTTP extension daemon should start");
        let stdout = child
            .stdout
            .take()
            .expect("the daemon readiness stream should be captured");
        let mut endpoint = String::new();
        tokio::time::timeout(
            Duration::from_secs(5),
            BufReader::new(stdout).read_line(&mut endpoint),
        )
        .await
        .expect("the daemon should announce its endpoint")
        .expect("the daemon endpoint should be readable");

        Self {
            child,
            endpoint: endpoint.trim().to_string(),
        }
    }

    async fn wait_for_exit(&mut self) {
        let status = tokio::time::timeout(Duration::from_secs(5), self.child.wait())
            .await
            .expect("the daemon should exit after session shutdown")
            .expect("the daemon exit status should be readable");
        assert!(status.success());
    }
}

fn http_fixture_path() -> PathBuf {
    let path = std::env::var_os("MEP_HTTP_FIXTURE")
        .map(PathBuf::from)
        .expect("MEP_HTTP_FIXTURE should point at the independently built HTTP extension daemon");
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
#[ignore = "requires the independently built mep-http-backend executable"]
async fn completes_mep_through_a_real_http_daemon() {
    let mut daemon = FixtureDaemon::start(&[]).await;
    let connection = DaemonConnection::new("mep-http-backend", &daemon.endpoint)
        .request_timeout(Duration::from_secs(2));
    let session = ConnectedDaemonSession::connect(connection)
        .expect("the HTTP extension client should be configured");

    support::mep::assert_backend_typestate_conformance(
        session,
        a_distribution_with_one_value(),
        json!("not Morphir IR"),
    )
    .await;

    daemon.wait_for_exit().await;
}

#[tokio::test]
#[ignore = "requires the independently built mep-http-backend executable"]
async fn carries_morphir_payloads_larger_than_jsonrpsee_defaults() {
    let mut daemon = FixtureDaemon::start(&[]).await;
    let connection = DaemonConnection::new("mep-http-backend", &daemon.endpoint)
        .request_timeout(Duration::from_secs(5));
    let session = ConnectedDaemonSession::connect(connection)
        .expect("the HTTP extension client should be configured");
    let session = session
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "large-payload-test".into(),
                version: "0.1.0".into(),
            },
        })
        .await
        .unwrap_or_else(|failure| panic!("initialization failed: {}", failure.error()));
    let large_ir = json!({ "padding": "x".repeat(11 * 1024 * 1024) });

    let (session, generated) = match session
        .invoke::<serde_json::Value>(
            methods::GENERATE,
            serde_json::to_value(GenerateRequest {
                ir: large_ir,
                options: Default::default(),
            })
            .expect("the generation request should serialize"),
        )
        .await
    {
        InvokeOutcome::Success(session, generated) => (session, generated),
        InvokeOutcome::Rejected(_, error) => panic!("generation was rejected: {error}"),
        InvokeOutcome::Failed(failure) => panic!("generation failed: {}", failure.error()),
    };

    assert_eq!(
        generated["artifacts"][0]["content"]
            .as_str()
            .expect("the backend should return the observed IR")
            .len(),
        11 * 1024 * 1024 + 14
    );
    session
        .shutdown()
        .await
        .unwrap_or_else(|failure| panic!("shutdown failed: {}", failure.error()));
    daemon.wait_for_exit().await;
}

#[tokio::test]
#[ignore = "requires the independently built mep-http-backend executable"]
async fn reports_connection_refusal_during_initialization() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("an isolated loopback port should be available");
    let endpoint = format!(
        "http://{}",
        listener
            .local_addr()
            .expect("the isolated address should be readable")
    );
    drop(listener);
    let connection = DaemonConnection::new("missing-http-backend", endpoint)
        .request_timeout(Duration::from_millis(250));
    let session = ConnectedDaemonSession::connect(connection)
        .expect("the endpoint URL should be valid even when nothing is listening");

    let failure = session
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "connection-failure-test".into(),
                version: "0.1.0".into(),
            },
        })
        .await
        .err()
        .expect("initialization should report the refused connection");

    assert!(
        failure
            .error()
            .to_string()
            .contains("HTTP extension request")
    );
    assert!(matches!(failure, FailedSession::Indeterminate(_, _)));
}

#[tokio::test]
#[ignore = "requires the independently built mep-http-backend executable"]
async fn marks_the_session_indeterminate_when_the_daemon_exceeds_the_request_timeout() {
    let mut daemon = FixtureDaemon::start(&["--hang-generate"]).await;
    let connection = DaemonConnection::new("mep-http-backend", &daemon.endpoint)
        .request_timeout(Duration::from_millis(100));
    let session = ConnectedDaemonSession::connect(connection)
        .expect("the HTTP extension client should be configured");
    let session = session
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "request-timeout-test".into(),
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

    assert!(
        failure
            .error()
            .to_string()
            .contains("HTTP extension request")
    );
    assert!(matches!(failure, FailedSession::Indeterminate(_, _)));
    daemon
        .child
        .kill()
        .await
        .expect("the hung daemon should stop");
    daemon
        .child
        .wait()
        .await
        .expect("the hung daemon should reap");
}

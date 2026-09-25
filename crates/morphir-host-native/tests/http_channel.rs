//! Conformance tests for a real extension reached over JSON-RPC HTTP.
//!
//! Build the fixture and provide its path before running these ignored tests:
//!
//! `cargo build -p morphir-host-native --features http --example mep-http-backend`
//! `MEP_HTTP_FIXTURE=target/debug/examples/mep-http-backend cargo test -p morphir-host-native --features http --test http_channel -- --ignored`
//!
//! `mise run test:http` does both.

mod support;

use morphir_extension_sdk::GenerateRequest;
use morphir_host::{CallError, ChannelCause, ChannelState, HostError, Session};
use morphir_host_native::http::{HttpChannel, HttpEndpoint};
use serde_json::json;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use support::mep::{backend_conformance, channel_state, completed, host_config};
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

    fn endpoint(&self, request_timeout: Duration) -> HttpEndpoint {
        HttpEndpoint {
            id: "mep-http-backend".into(),
            url: self.endpoint.clone(),
            request_timeout,
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

fn connect(endpoint: HttpEndpoint) -> HttpChannel {
    HttpChannel::connect(endpoint).expect("the HTTP extension client should be configured")
}

#[tokio::test]
#[ignore = "requires the independently built mep-http-backend executable"]
async fn completes_mep_through_a_real_http_daemon() {
    let mut daemon = FixtureDaemon::start(&[]).await;
    let channel = connect(daemon.endpoint(Duration::from_secs(2)));

    backend_conformance(
        channel.connection(),
        "json",
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
    let channel = connect(daemon.endpoint(Duration::from_secs(5)));
    let mut session = Session::open(
        channel.connection(),
        &host_config("large-payload-test", "0.1.0"),
    )
    .await
    .unwrap_or_else(|error| panic!("initialization failed: {error}"));
    let large_ir = json!({ "padding": "x".repeat(11 * 1024 * 1024) });

    let generated = completed(
        "generation",
        session
            .generate(GenerateRequest {
                ir: large_ir,
                target: "json".into(),
                options: Default::default(),
            })
            .await,
    );

    assert_eq!(generated.artifacts[0].content.len(), 11 * 1024 * 1024 + 14);
    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("shutdown failed: {error}"));
    daemon.wait_for_exit().await;
}

#[tokio::test]
#[ignore = "requires the independently built mep-http-backend executable"]
async fn reports_connection_refusal_during_initialization() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("an isolated loopback port should be available");
    let url = format!(
        "http://{}",
        listener
            .local_addr()
            .expect("the isolated address should be readable")
    );
    drop(listener);
    let channel = HttpChannel::connect(HttpEndpoint {
        id: "missing-http-backend".into(),
        url,
        request_timeout: Duration::from_millis(250),
    })
    .expect("the endpoint URL should be valid even when nothing is listening");

    let error = Session::open(
        channel.connection(),
        &host_config("connection-failure-test", "0.1.0"),
    )
    .await
    .err()
    .expect("initialization should report the refused connection");

    assert!(error.to_string().contains("HTTP extension request"));
    assert_eq!(channel_state(&error), ChannelState::Indeterminate);
}

#[tokio::test]
#[ignore = "requires the independently built mep-http-backend executable"]
async fn marks_the_session_indeterminate_when_the_daemon_exceeds_the_request_timeout() {
    let mut daemon = FixtureDaemon::start(&["--hang-generate"]).await;
    let channel = connect(daemon.endpoint(Duration::from_millis(100)));
    let mut session = Session::open(
        channel.connection(),
        &host_config("request-timeout-test", "0.1.0"),
    )
    .await
    .unwrap_or_else(|error| panic!("initialization failed: {error}"));

    let error = match session
        .generate(GenerateRequest {
            ir: a_distribution_with_one_value(),
            target: "json".into(),
            options: Default::default(),
        })
        .await
    {
        Err(CallError::Failed(error)) => error,
        Ok(_) => panic!("the hung request should time out"),
        Err(other) => panic!("the hung request should fail the session, got {other:?}"),
    };

    assert!(error.to_string().contains("HTTP extension request"));
    assert!(matches!(
        error,
        HostError::Channel {
            state: ChannelState::Indeterminate,
            cause: ChannelCause::Transport,
            ..
        }
    ));
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

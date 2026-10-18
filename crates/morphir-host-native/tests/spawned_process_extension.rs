//! Conformance tests for a real native extension child process.
//!
//! Build the fixture and provide its path before running this ignored test:
//!
//! `cargo build -p morphir-host-native --example mep-native-backend`
//! `MEP_NATIVE_FIXTURE=target/debug/examples/mep-native-backend cargo test -p morphir-host-native --test spawned_process_extension -- --ignored`

mod support;

use base64::Engine as _;
use morphir_extension_sdk::model::{
    GenerateModelRequest, MODEL_MEP_VERSION, ModelTransportSupport,
};
use morphir_extension_sdk::{GenerateRequest, protocol::methods};
use morphir_host::HostError;
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
async fn transports_exact_64_mib_generated_binary_and_worst_case_json_text() {
    use morphir_distribution::Sha256Digest;
    use morphir_host::GuestResponsePolicy;
    // Frozen independently authored chunk-indexed byte recipes, crosschecked
    // with OpenSSL before the fixture producer was implemented.
    let hashes = [
        "728f52a85b49e21b090f8aab42eac87b30f1c82bdb3a3890627bc244de6756b0",
        "d42312f945214c8f55c283c08415efdddb975c664bc7ae4b129a6c37b79b87c8",
        "75e9210fb9b010ccd02fc3b824d482d6f1646aa3a5765baba3034423eb1550f7",
        "d5f3e8e4c2a22852ec5d5aa01c92f7ad149031ea896fd3fd70887f425fc30d27",
    ];
    for kind in ["binary", "escaped-text"] {
        let process = SharedProcess::spawn_with_response_policy(
            native_launch().request_timeout(Duration::from_secs(120)),
            GuestResponsePolicy::GeneratedArtifacts,
        )
        .await;
        let mut session = Session::open(
            process.connection().await,
            &host_config("artifact-byte-test", "0.1.0"),
        )
        .await
        .unwrap();
        let result = session
            .generate(GenerateRequest {
                ir: a_distribution_with_one_value(),
                target: "json".into(),
                options: [("artifact-response".into(), json!(kind))].into(),
            })
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.artifacts.len(), 4);
        let mut total = 0;
        for (index, artifact) in result.artifacts.iter().enumerate() {
            let bytes = if kind == "binary" {
                assert!(artifact.binary);
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&artifact.content)
                    .unwrap();
                assert_eq!(Sha256Digest::of_bytes(&bytes).to_string(), hashes[index]);
                bytes
            } else {
                assert!(!artifact.binary);
                assert!(artifact.content.bytes().all(|byte| byte == 0));
                artifact.content.as_bytes().to_vec()
            };
            assert_eq!(bytes.len(), 16 * 1024 * 1024);
            total += bytes.len();
        }
        assert_eq!(total, 64 * 1024 * 1024);
        // Reuse stays negotiated and a following ordinary generation is intact.
        let small = session
            .generate(GenerateRequest {
                ir: a_distribution_with_one_value(),
                target: "json".into(),
                options: Default::default(),
            })
            .await
            .unwrap();
        assert_eq!(small.artifacts[0].path, "observed-ir.json");
        session.close().await.unwrap();
        assert!(!process.is_running().await);
        assert!(process.stdout_is_exhausted().await);
    }
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn legacy_sessions_refuse_large_artifacts_and_artifact_timeouts_reap_the_child() {
    use morphir_host::GuestResponsePolicy;
    for (policy, hang, marker) in [
        (GuestResponsePolicy::Legacy, false, "67108864 byte limit"),
        (GuestResponsePolicy::GeneratedArtifacts, true, "timed out"),
    ] {
        let launch = if hang {
            native_launch()
                .env("MEP_FIXTURE_HANG_GENERATE", "1")
                .request_timeout(Duration::from_secs(1))
        } else {
            native_launch().request_timeout(Duration::from_secs(120))
        };
        let process = SharedProcess::spawn_with_response_policy(launch, policy).await;
        let mut session = Session::open(
            process.connection().await,
            &host_config("artifact-refusal-test", "0.1.0"),
        )
        .await
        .unwrap();
        let error = session
            .generate(GenerateRequest {
                ir: a_distribution_with_one_value(),
                target: "json".into(),
                options: [("artifact-response".into(), json!("binary"))].into(),
            })
            .await
            .unwrap_err();
        match error {
            CallError::Failed(error) => {
                assert!(error.to_string().contains(marker), "{error}");
                assert_eq!(channel_state(&error), ChannelState::Stopped);
            }
            other => panic!("expected a stopped transport, got {other:?}"),
        }
        assert!(!process.is_running().await);
    }
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn retains_exact_ion_models_through_a_real_child_process() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../morphir-extension-sdk/tests/fixtures/model_transport/model-transport.cases.json"
    ))
    .unwrap();
    let reference = corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["id"] == "model-0001")
        .unwrap();
    let offer: ModelTransportSupport =
        serde_json::from_value(reference["hostOffer"]["modelTransport"].clone()).unwrap();
    let mut request: GenerateModelRequest =
        serde_json::from_value(reference["request"]["params"].clone()).unwrap();
    request.target = "json".into();
    let original = request.model.payload.clone();
    let process = SharedProcess::spawn(native_launch().env("MEP_FIXTURE_MODEL", "1")).await;
    let config = host_config("model-process-test", "0.1.0").with_model_transport(offer);
    let mut session = Session::open(process.connection().await, &config)
        .await
        .unwrap();
    assert_eq!(session.negotiated().protocol_version(), MODEL_MEP_VERSION);

    let result = session.generate_model(request.clone()).await.unwrap();
    assert!(result.success);
    assert_eq!(result.artifacts.len(), 1);
    assert!(result.artifacts[0].binary);
    assert_eq!(result.artifacts[0].content, original.content);
    let observed = base64::engine::general_purpose::STANDARD
        .decode(&result.artifacts[0].content)
        .unwrap();
    assert_eq!(
        observed,
        include_bytes!(
            "../../morphir-extension-sdk/tests/fixtures/model_transport/fixtures/rich-model.ionb"
        )
    );

    request.model.payload.sha256 = "0".repeat(64);
    assert!(matches!(
        session.generate_model(request).await,
        Err(CallError::Rejected(HostError::Rejected(_)))
    ));
    assert!(process.is_running().await);
    let legacy = session
        .generate(GenerateRequest {
            ir: a_distribution_with_one_value(),
            target: "json".into(),
            options: Default::default(),
        })
        .await
        .unwrap();
    assert!(legacy.success);
    assert_eq!(legacy.artifacts[0].path, "observed-ir.json");
    session.close().await.unwrap();
    assert!(!process.is_running().await);
    assert!(process.stdout_is_exhausted().await);
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

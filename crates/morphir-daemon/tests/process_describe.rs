//! Run with the independently built `mep-native-backend` fixture, as in
//! `spawned_process_extension.rs`.

use morphir_daemon::extensions::process::DescriptionSource;
use morphir_daemon::extensions::{ProcessLaunch, SpawnedProcessTransport};
use morphir_extension_sdk::protocol::{InitializeParams, PeerInfo};
use std::{path::PathBuf, time::Duration};

struct DescribeDriver;
impl DescribeDriver {
    async fn describe(
        mode: &str,
    ) -> morphir_daemon::Result<morphir_daemon::extensions::process::ProcessDescription> {
        let fixture =
            PathBuf::from(std::env::var_os("MEP_NATIVE_FIXTURE").expect("set MEP_NATIVE_FIXTURE"));
        let fixture = if fixture.is_absolute() {
            fixture
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(fixture)
        };
        let launch = ProcessLaunch::new(
            "mep-native-backend",
            fixture,
            std::env::current_dir().unwrap(),
        )
        .request_timeout(Duration::from_secs(2));
        let launch = if mode == "plain" {
            launch
        } else {
            launch.env("MEP_FIXTURE_DESCRIBE", mode)
        };
        SpawnedProcessTransport::spawn(launch)
            .await?
            .describe(InitializeParams {
                protocol_versions: vec!["0.1".into(), "future".into()],
                host: PeerInfo {
                    kind: Default::default(),
                    name: "describe-test".into(),
                    version: "0.2.0".into(),
                },
            })
            .await
    }
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn describes_a_guest_before_initialize_and_exits() {
    let result = DescribeDriver::describe("describe").await.unwrap();
    assert_eq!(result.source, DescriptionSource::Describe);
    assert_eq!(result.statement.extension.id, "mep-native-backend");
    assert_eq!(result.statement.capabilities["backend"]["generate"], true);
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn falls_back_on_method_not_found() {
    assert_fallback("method-not-found").await;
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn falls_back_on_pre_initialize_refusal() {
    assert_fallback("not-initialized").await;
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn falls_back_on_a_legacy_pre_initialize_refusal_message() {
    assert_fallback("legacy-not-initialized").await;
}

async fn assert_fallback(mode: &str) {
    let result = DescribeDriver::describe(mode).await.unwrap();
    assert_eq!(result.source, DescriptionSource::SessionFallback);
    assert_eq!(result.statement.protocol_versions, ["0.1"]);
    let wire = serde_json::to_value(result.statement).unwrap();
    assert!(wire.get("requires").is_none());
    assert!(wire.get("critical").is_none());
    assert_eq!(wire["capabilities"]["backend"]["future"], "preserved");
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn unrelated_errors_and_invalid_statements_do_not_fall_back() {
    for (mode, expected) in [
        ("internal-error", "deliberate failure"),
        ("critical", "capabilities.backend.future"),
        ("version", "statementVersion"),
        ("wrong-id", "identity"),
        ("requires-host", "requires.host"),
    ] {
        let error = DescribeDriver::describe(mode).await.unwrap_err();
        assert!(error.to_string().contains(expected), "{mode}: {error}");
    }
}

#[tokio::test]
#[ignore = "requires the independently built mep-native-backend executable"]
async fn describes_the_unmodified_sdk_guest() {
    let result = DescribeDriver::describe("plain").await.unwrap();
    assert_eq!(result.source, DescriptionSource::Describe);
}

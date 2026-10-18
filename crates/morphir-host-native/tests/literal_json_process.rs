//! Build mep-literal-json-backend and set MEP_LITERAL_JSON_FIXTURE to run.

use morphir_extension_sdk::protocol::ExtensionResponse;
use morphir_host_native::process::{ProcessChild, ProcessLaunch};
use std::time::Duration;

async fn exchange(child: &mut ProcessChild, body: &str) -> ExtensionResponse {
    // RawValue preserves escaped member spellings through production framing.
    let raw = serde_json::value::RawValue::from_string(body.into()).unwrap();
    child.write(&raw).await.unwrap();
    let bytes = child.read().await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
#[ignore = "requires the independently built mep-literal-json-backend executable"]
async fn literal_wire_json_reaches_a_real_sdk_process_backend_without_reinterpretation() {
    let path = std::path::PathBuf::from(
        std::env::var_os("MEP_LITERAL_JSON_FIXTURE").expect("set MEP_LITERAL_JSON_FIXTURE"),
    );
    let path = if path.is_absolute() {
        path
    } else {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    };
    let launch = ProcessLaunch::new(
        "mep-literal-json-backend",
        path,
        std::env::current_dir().unwrap(),
    )
    .request_timeout(Duration::from_secs(5));
    let mut child = ProcessChild::spawn(&launch).await.unwrap();
    let initialized = exchange(
        &mut child,
        r#"{"jsonrpc":"2.0","id":1,"method":"morphir.initialize","params":{"protocolVersions":["0.1"],"host":{"name":"literal-process","version":"1"}}}"#,
    ).await;
    assert!(initialized.error.is_none(), "{:?}", initialized.error);
    let response = exchange(
        &mut child,
        r#"{"jsonrpc":"2.0","id":2,"method":"morphir.backend.generate","params":{"ir":{"nested":[{"\u0024serde_json::private::Number":"1"},{"\u0024serde_json::private::RawValue":"{\"x\":1,\"x\":2}"},18446744073709551615,0.123456789012345678901234567890]},"target":"literal-json","options":{"literal":{"\u0024serde_json::private::RawValue":"true"}}}}"#,
    ).await;
    assert!(response.error.is_none(), "{:?}", response.error);
    let artifacts = response.result.unwrap()["artifacts"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(
        artifacts[0]["content"],
        r#"{"nested":[{"$serde_json::private::Number":"1"},{"$serde_json::private::RawValue":"{\"x\":1,\"x\":2}"},18446744073709551615,0.123456789012345678901234567890]}"#
    );
    assert_eq!(
        artifacts[1]["content"],
        r#"{"$serde_json::private::RawValue":"true"}"#
    );
    let shutdown = exchange(
        &mut child,
        r#"{"jsonrpc":"2.0","id":3,"method":"morphir.shutdown","params":{}}"#,
    )
    .await;
    assert!(shutdown.error.is_none());
    let exit = r#"{"jsonrpc":"2.0","method":"morphir.exit"}"#;
    child
        .write(&serde_json::value::RawValue::from_string(exit.into()).unwrap())
        .await
        .unwrap();
    assert!(child.wait_for_exit().await.unwrap().success());
    assert!(
        child.stdout_is_exhausted().await.unwrap(),
        "stdout contains only protocol frames"
    );
}

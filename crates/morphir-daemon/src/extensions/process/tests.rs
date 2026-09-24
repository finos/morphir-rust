use super::*;
use morphir_extension_sdk::{
    BackendCapability, ExtensionCapabilities, ExtensionInfo, ExtensionType,
};

#[test]
fn compatibility_initialization_rejects_locked_backend_capability_drift() {
    let discovered = ExtensionInfo {
        id: "example".into(),
        name: "Example".into(),
        version: "1.0.0".into(),
        types: vec![ExtensionType::Backend],
        ..ExtensionInfo::default()
    };
    let locked_backend = BackendCapability {
        targets: vec!["avro".into()],
        ir_versions: vec!["3".into(), "4.0.0".into()],
        generate: true,
    };
    let expected = ExpectedExtension::discovered_with_capabilities(
        discovered.clone(),
        ExtensionCapabilities {
            backend: Some(locked_backend.clone()),
            ..ExtensionCapabilities::default()
        },
    );
    let initialized = InitializeResult {
        protocol_version: "0.1".into(),
        extension: discovered,
        capabilities: ExtensionCapabilities {
            backend: Some(BackendCapability {
                generate: false,
                ..locked_backend
            }),
            ..ExtensionCapabilities::default()
        },
    };

    let error = validate_compatibility_initialization(expected, &["0.1".into()], initialized)
        .expect_err("compatibility sessions must enforce discovery-time capability locks");

    assert!(
        error
            .to_string()
            .contains("backend capabilities disagreed with discovery"),
        "{error}"
    );
}

#[test]
fn compatibility_negotiation_retains_disabled_generate_support() {
    let extension = ExtensionInfo {
        id: "example".into(),
        name: "Example".into(),
        version: "1.0.0".into(),
        types: vec![ExtensionType::Backend],
        ..ExtensionInfo::default()
    };
    let initialized = InitializeResult {
        protocol_version: "0.1".into(),
        extension: extension.clone(),
        capabilities: ExtensionCapabilities {
            backend: Some(BackendCapability {
                targets: vec!["avro".into()],
                ir_versions: vec!["4".into()],
                generate: false,
            }),
            ..ExtensionCapabilities::default()
        },
    };

    let negotiated = validate_compatibility_initialization(
        ExpectedExtension::discovered(extension),
        &["0.1".into()],
        initialized,
    )
    .unwrap();

    assert!(!negotiated.supports_method(methods::GENERATE));
}

#[tokio::test]
async fn compatibility_invoke_rejects_unsafe_generated_artifacts() {
    let error = validate_compatibility_method_result(
        methods::GENERATE,
        serde_json::json!({}),
        serde_json::json!({
            "success": true,
            "artifacts": [{"path": "../../escape.avsc", "content": "{}"}],
            "diagnostics": []
        }),
    )
    .await
    .expect_err("compatibility results must use the shared artifact validator");

    assert!(error.to_string().contains("artifact path"), "{error}");
}

#[test]
fn stderr_capture_retains_only_the_bounded_tail() {
    let mut output = b"old diagnostics".to_vec();
    append_bounded_tail(&mut output, b"new diagnostics", 16);

    assert_eq!(output, b"snew diagnostics");

    append_bounded_tail(&mut output, b"0123456789abcdefghijkl", 16);
    assert_eq!(output, b"6789abcdefghijkl");
}

/// A reader whose first poll always fails, to force a real `std::io::Error`
/// out of `read_frame` without relying on process teardown timing.
struct FailingReader;

impl tokio::io::AsyncRead for FailingReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        _buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Err(std::io::Error::other("stdout pipe broke")))
    }
}

#[tokio::test]
async fn an_io_failure_from_read_frame_keeps_the_daemons_io_error_text() {
    let mut reader = BufReader::new(FailingReader);
    let error = read_frame(&mut reader)
        .await
        .expect_err("a broken pipe must surface as an error, not hang or panic");

    let daemon_error = DaemonError::from(error);

    assert!(matches!(daemon_error, DaemonError::Io(_)), "{daemon_error}");
    assert!(
        daemon_error.to_string().starts_with("IO error: "),
        "{daemon_error}"
    );
}

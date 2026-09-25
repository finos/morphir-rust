use morphir_extension_sdk::protocol::InitializeResult;
use morphir_extension_sdk::protocol::{ExtensionResponse, PeerInfo, PeerKind, methods};
use morphir_extension_sdk::{
    BackendCapability, ExtensionCapabilities, ExtensionInfo, ExtensionType, GenerateRequest,
};
use morphir_host::testing::MemoryChannel;
use morphir_host::{
    BasicChecks, CallError, ChannelCause, ChannelError, ChannelState, GuestConnection, HostConfig,
    HostError, JsonRpcConnection, Pool, Session,
};
use morphir_host_native::CheckedConnection;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn config() -> HostConfig {
    HostConfig::new(PeerInfo {
        kind: PeerKind::Unspecified,
        name: "test".into(),
        version: "1.0.0".into(),
    })
}

fn backend() -> InitializeResult {
    InitializeResult {
        protocol_version: "0.1".into(),
        extension: ExtensionInfo {
            id: "guest".into(),
            name: "Guest".into(),
            version: "1.0.0".into(),
            types: vec![ExtensionType::Backend],
            ..ExtensionInfo::default()
        },
        capabilities: ExtensionCapabilities {
            backend: Some(BackendCapability {
                targets: vec!["x".into()],
                ir_versions: vec!["4".into()],
                generate: true,
            }),
            ..ExtensionCapabilities::default()
        },
    }
}

fn generate_request() -> GenerateRequest {
    GenerateRequest {
        ir: json!({}),
        target: "x".into(),
        options: Default::default(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_traversal_artifact_ends_the_session() {
    let channel = MemoryChannel::new()
        .respond(ExtensionResponse::success(1, backend()).unwrap())
        .respond(
            ExtensionResponse::success(
                2,
                json!({
                    "success": true,
                    "artifacts": [{"path": "../../escape.avsc", "content": "{}"}],
                    "diagnostics": []
                }),
            )
            .unwrap(),
        )
        .respond(ExtensionResponse::success(3, json!({})).unwrap());
    let log = channel.log();
    let connection =
        CheckedConnection::new(JsonRpcConnection::new(channel, BasicChecks::new("guest")));
    let mut session = Session::open(connection, &config()).await.unwrap();

    let error = session.generate(generate_request()).await.unwrap_err();

    let CallError::Invalid(error) = error else {
        panic!("a rejected artifact must end the session: {error:?}");
    };
    assert_eq!(
        error.to_string(),
        "Generated artifact path '../../escape.avsc' is invalid: invalid local artifact path \
         \"../../escape.avsc\": expected a normalized relative path of at most 4096 UTF-8 bytes \
         and 1024 UTF-16 units with portable filename components"
    );
    assert_eq!(
        log.methods(),
        [
            methods::INITIALIZE,
            methods::GENERATE,
            methods::SHUTDOWN,
            methods::EXIT
        ]
    );
    assert_eq!(log.closes(), 1);
    assert_eq!(log.aborts(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_shutdown_after_a_rejected_result_names_both_failures() {
    let channel = MemoryChannel::new()
        .respond(ExtensionResponse::success(1, backend()).unwrap())
        .respond(
            ExtensionResponse::success(
                2,
                json!({
                    "success": true,
                    "artifacts": [{"path": "schema.avro", "content": "not base64!", "binary": true}],
                    "diagnostics": []
                }),
            )
            .unwrap(),
        )
        .fail(ChannelError {
            message: "pipe closed".into(),
            state: ChannelState::Indeterminate,
            cause: ChannelCause::Transport,
        });
    let connection =
        CheckedConnection::new(JsonRpcConnection::new(channel, BasicChecks::new("guest")));
    let mut session = Session::open(connection, &config()).await.unwrap();

    let error = session.generate(generate_request()).await.unwrap_err();

    let CallError::Invalid(HostError::Channel { message, state, .. }) = error else {
        panic!("a failed shutdown keeps its channel state: {error:?}");
    };
    assert_eq!(state, ChannelState::Indeterminate);
    assert!(
        message.starts_with("Generated binary artifact 'schema.avro' contains invalid Base64: "),
        "{message}"
    );
    assert!(
        message.ends_with("; orderly shutdown also failed: pipe closed"),
        "{message}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_valid_result_passes_through_and_keeps_the_session_ready() {
    let channel = MemoryChannel::new()
        .respond(ExtensionResponse::success(1, backend()).unwrap())
        .respond(
            ExtensionResponse::success(
                2,
                json!({
                    "success": true,
                    "artifacts": [{"path": "out/Main.x", "content": "main"}],
                    "diagnostics": []
                }),
            )
            .unwrap(),
        )
        .respond(ExtensionResponse::success(3, json!({})).unwrap());
    let log = channel.log();
    let connection =
        CheckedConnection::new(JsonRpcConnection::new(channel, BasicChecks::new("guest")));
    let mut session = Session::open(connection, &config()).await.unwrap();

    let result = session.generate(generate_request()).await.unwrap();
    session.close().await.unwrap();

    assert_eq!(result.artifacts[0].path, "out/Main.x");
    assert_eq!(
        log.methods().last().map(String::as_str),
        Some(methods::EXIT)
    );
    assert_eq!(log.closes(), 1);
}

// A result that fails the host's checks is a deterministic bad answer: the
// same guest build would give it again. The pool reports it once, with the
// check's own text, and opens no second guest to ask again.
#[tokio::test(flavor = "multi_thread")]
async fn a_pooled_result_that_fails_a_check_is_not_retried() {
    let opens = Arc::new(AtomicUsize::new(0));
    let open = {
        let opens = Arc::clone(&opens);
        move || {
            opens.fetch_add(1, Ordering::SeqCst);
            let channel = MemoryChannel::new()
                .respond(ExtensionResponse::success(1, backend()).unwrap())
                .respond(
                    ExtensionResponse::success(
                        2,
                        json!({
                            "success": true,
                            "artifacts": [{"path": "../../escape.avsc", "content": "{}"}],
                            "diagnostics": []
                        }),
                    )
                    .unwrap(),
                )
                .respond(ExtensionResponse::success(3, json!({})).unwrap());
            let connection: Box<dyn GuestConnection> = Box::new(CheckedConnection::new(
                JsonRpcConnection::new(channel, BasicChecks::new("guest")),
            ));
            std::future::ready(Ok(connection))
        }
    };
    let pool: Pool<String> = Pool::new(config());

    let error = pool
        .call::<_, serde_json::Value, _, _>(
            &"provider".to_owned(),
            "fp",
            open,
            methods::GENERATE,
            &generate_request(),
        )
        .await
        .unwrap_err();

    let CallError::Invalid(error) = error else {
        panic!("a result that fails a check is Invalid: {error:?}");
    };
    assert_eq!(
        error.to_string(),
        "Generated artifact path '../../escape.avsc' is invalid: invalid local artifact path \
         \"../../escape.avsc\": expected a normalized relative path of at most 4096 UTF-8 bytes \
         and 1024 UTF-16 units with portable filename components"
    );
    assert_eq!(
        opens.load(Ordering::SeqCst),
        1,
        "a failed check opens no second guest"
    );
}

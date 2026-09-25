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

// Compile and workspace result checks, ported from the daemon's session tests.

mod results {
    use super::config;
    use morphir_extension_sdk::protocol::{ExtensionResponse, InitializeResult, methods};
    use morphir_extension_sdk::{
        CompileResult, ExtensionCapabilities, ExtensionInfo, ExtensionType, FrontendCapability,
        WorkspaceCapability,
    };
    use morphir_host::testing::{MemoryChannel, SentLog};
    use morphir_host::{BasicChecks, CallError, JsonRpcConnection, Session};
    use morphir_host_native::CheckedConnection;
    use serde_json::json;

    fn extension(types: Vec<ExtensionType>) -> ExtensionInfo {
        ExtensionInfo {
            id: "example".into(),
            name: "Example".into(),
            version: "1.0.0".into(),
            types,
            ..Default::default()
        }
    }

    fn frontend_initialization() -> InitializeResult {
        InitializeResult {
            protocol_version: "0.1".into(),
            extension: extension(vec![ExtensionType::Frontend]),
            capabilities: ExtensionCapabilities {
                frontend: Some(FrontendCapability {
                    compile: true,
                    ..FrontendCapability::default()
                }),
                ..ExtensionCapabilities::default()
            },
        }
    }

    fn workspace_initialization() -> InitializeResult {
        InitializeResult {
            protocol_version: "0.1".into(),
            extension: extension(vec![ExtensionType::Workspace]),
            capabilities: ExtensionCapabilities {
                workspace: Some(WorkspaceCapability {
                    protocol_versions: vec![
                        morphir_workspace::Version::parse("0.1.0-draft.1").unwrap(),
                    ],
                    discover: true,
                }),
                ..ExtensionCapabilities::default()
            },
        }
    }

    fn compile_params(ir_version: &str) -> serde_json::Value {
        json!({
            "languageId": "elm",
            "sources": {"documents": []},
            "package": {"name": "example/package", "exposedModules": []},
            "dependencies": [],
            "options": {"typesOnly": false, "irVersion": ir_version}
        })
    }

    /// A checked session on a guest that answers `initialized`, then
    /// `result` for the call, then the shutdown.
    async fn open(initialized: InitializeResult, result: serde_json::Value) -> (Session, SentLog) {
        let channel = MemoryChannel::new()
            .respond(ExtensionResponse::success(1, initialized).unwrap())
            .respond(ExtensionResponse::success(2, result).unwrap())
            .respond(ExtensionResponse::success(3, json!({})).unwrap());
        let log = channel.log();
        let connection =
            CheckedConnection::new(JsonRpcConnection::new(channel, BasicChecks::new("example")));
        let session = Session::open(connection, &config())
            .await
            .unwrap_or_else(|error| panic!("initialization failed: {error}"));
        (session, log)
    }

    /// The error of a call whose result must fail the host's checks. The
    /// session is closed in order.
    fn invalid<T: std::fmt::Debug>(result: Result<T, CallError>, log: &SentLog) -> String {
        match result {
            Err(CallError::Invalid(error)) => {
                assert_eq!(log.closes(), 1, "the session is closed in order");
                assert_eq!(
                    log.methods().last().map(String::as_str),
                    Some(methods::EXIT)
                );
                error.to_string()
            }
            other => panic!("a result that fails a check must end the session: {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rejects_successful_compile_result_without_ir_version() {
        let (mut session, log) = open(
            frontend_initialization(),
            json!({"success": true, "ir": {}, "diagnostics": [], "modules": []}),
        )
        .await;

        let result = session
            .call::<_, CompileResult>(methods::COMPILE, compile_params("3"))
            .await;

        let message = invalid(result, &log);
        assert!(message.contains("missing irVersion"), "{message}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rejects_successful_compile_result_without_ir_for_raw_callers() {
        let (mut session, log) = open(
            frontend_initialization(),
            json!({"success": true, "irVersion": "3", "diagnostics": [], "modules": []}),
        )
        .await;

        let result = session
            .call::<_, serde_json::Value>(methods::COMPILE, json!({}))
            .await;

        let message = invalid(result, &log);
        assert!(message.contains("missing ir"), "{message}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn accepts_successful_compile_result_with_ir_version_and_ir() {
        let (mut session, _) = open(
            frontend_initialization(),
            json!({
                "success": true,
                "irVersion": "3",
                "ir": {
                    "formatVersion": 3,
                    "distribution": ["Library", [], [], {"modules": []}]
                },
                "diagnostics": [],
                "modules": ["Example"]
            }),
        )
        .await;

        let result = session
            .call::<_, CompileResult>(methods::COMPILE, compile_params("3"))
            .await
            .unwrap_or_else(|error| panic!("valid success failed the session: {error:?}"));

        assert!(result.success);
        assert_eq!(result.ir_version.as_deref(), Some("3"));
        assert!(result.ir.is_some());
        session.close().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rejects_a_successful_compile_result_for_another_requested_ir_version() {
        let (mut session, log) = open(
            frontend_initialization(),
            json!({
                "success": true,
                "irVersion": "4.0.0",
                "ir": {"Library": {}},
                "diagnostics": [],
                "modules": []
            }),
        )
        .await;

        let result = session
            .call::<_, CompileResult>(methods::COMPILE, compile_params("3"))
            .await;

        let message = invalid(result, &log);
        assert!(
            message.contains("did not match requested irVersion"),
            "{message}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn malformed_workspace_discovery_result_fails_the_session() {
        let (mut session, log) = open(
            workspace_initialization(),
            json!({
                "status": "success",
                "snapshot": {"protocolVersion": "0.1.0-draft.1"}
            }),
        )
        .await;

        let result = session
            .call::<_, serde_json::Value>(
                methods::WORKSPACE_DISCOVER,
                json!({
                    "protocolVersion": "0.1.0-draft.1",
                    "developmentRoot": {"entries": {}},
                    "morphirHome": null,
                    "systemConfig": null,
                    "environment": {},
                    "cliOverlay": {}
                }),
            )
            .await;

        let message = invalid(result, &log);
        assert!(message.contains("missing field"), "{message}");
    }
}

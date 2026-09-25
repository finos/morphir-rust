use morphir_extension_sdk::protocol::{ExtensionResponse, PeerInfo, PeerKind, RpcError, methods};
use morphir_extension_sdk::{CompileRequest, CompileResult};
use morphir_host::testing::{MemoryChannel, frontend_initialize_result};
use morphir_host::{
    BasicChecks, CallError, ChannelCause, ChannelError, ChannelState, HostConfig, HostError,
    JsonRpcConnection, Session, compile_once,
};
use serde_json::json;

fn config() -> HostConfig {
    HostConfig::new(PeerInfo {
        kind: PeerKind::Unspecified,
        name: "test".into(),
        version: "1.0.0".into(),
    })
}

fn ok(id: u64, value: impl serde::Serialize) -> ExtensionResponse {
    ExtensionResponse::success(id, value).unwrap()
}

fn compiled() -> CompileResult {
    CompileResult {
        success: true,
        ir_version: Some("4.0.0".into()),
        ir: Some(json!({})),
        diagnostics: Vec::new(),
        modules: vec!["Example".into()],
        module_results: Vec::new(),
        context_digest: None,
    }
}

#[tokio::test]
async fn compile_once_opens_compiles_and_closes() {
    let channel = MemoryChannel::new()
        .respond(ok(1, frontend_initialize_result("guest")))
        .respond(ok(2, compiled()))
        .respond(ok(3, json!({})));
    let log = channel.log();
    let connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));

    let result = compile_once(connection, &config(), CompileRequest::default())
        .await
        .unwrap();

    assert!(result.success);
    assert_eq!(result.modules, ["Example"]);
    assert_eq!(
        log.methods(),
        [
            methods::INITIALIZE,
            methods::COMPILE,
            methods::SHUTDOWN,
            methods::EXIT
        ]
    );
}

#[tokio::test]
async fn a_session_serves_many_calls() {
    let channel = MemoryChannel::new()
        .respond(ok(1, frontend_initialize_result("guest")))
        .respond(ok(2, compiled()))
        .respond(ok(3, compiled()))
        .respond(ok(4, json!({})));
    let log = channel.log();
    let connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    let mut session = Session::open(connection, &config()).await.unwrap();

    assert_eq!(session.negotiated().extension().id, "guest");
    session.compile(CompileRequest::default()).await.unwrap();
    session.compile(CompileRequest::default()).await.unwrap();
    session.close().await.unwrap();

    assert_eq!(
        log.methods(),
        [
            methods::INITIALIZE,
            methods::COMPILE,
            methods::COMPILE,
            methods::SHUTDOWN,
            methods::EXIT
        ]
    );
}

#[tokio::test]
async fn compile_once_reports_a_rejection_after_an_orderly_shutdown() {
    let channel = MemoryChannel::new()
        .respond(ok(1, frontend_initialize_result("guest")))
        .respond(ExtensionResponse::error(
            2,
            RpcError {
                code: -32001,
                message: "does not compile".into(),
                data: None,
            },
        ))
        .respond(ok(3, json!({})));
    let log = channel.log();
    let connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));

    let error = compile_once(connection, &config(), CompileRequest::default())
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "RPC error -32001: does not compile");
    assert_eq!(
        log.methods().last().map(String::as_str),
        Some(methods::EXIT)
    );
}

#[tokio::test]
async fn a_result_that_does_not_decode_ends_the_session_in_order() {
    let channel = MemoryChannel::new()
        .respond(ok(1, frontend_initialize_result("guest")))
        .respond(ok(2, json!({"not": "a compile result"})))
        .respond(ok(3, json!({})));
    let log = channel.log();
    let connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));

    compile_once(connection, &config(), CompileRequest::default())
        .await
        .unwrap_err();

    assert_eq!(
        log.methods(),
        [
            methods::INITIALIZE,
            methods::COMPILE,
            methods::SHUTDOWN,
            methods::EXIT
        ]
    );
    assert_eq!(log.closes(), 1);
}

#[tokio::test]
async fn closing_after_a_decode_failure_closes_the_channel_once() {
    let channel = MemoryChannel::new()
        .respond(ok(1, frontend_initialize_result("guest")))
        .respond(ok(2, json!({"not": "a compile result"})))
        .respond(ok(3, json!({})));
    let log = channel.log();
    let connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    let mut session = Session::open(connection, &config()).await.unwrap();

    match session.compile(CompileRequest::default()).await {
        Err(CallError::Invalid(_)) => {}
        other => panic!("expected a decode failure, got {other:?}"),
    }

    session.close().await.unwrap();

    assert_eq!(log.closes(), 1);
    assert_eq!(
        log.methods(),
        [
            methods::INITIALIZE,
            methods::COMPILE,
            methods::SHUTDOWN,
            methods::EXIT
        ]
    );
}

// A result that does not decode still ends the session in order. When that
// shutdown fails too, the error names both failures and keeps the channel
// state the shutdown failure proves.
#[tokio::test]
async fn a_failed_shutdown_after_a_result_that_does_not_decode_names_both_failures() {
    let channel = MemoryChannel::new()
        .respond(ok(1, frontend_initialize_result("guest")))
        .respond(ok(2, json!({"not": "a compile result"})))
        .fail(ChannelError {
            message: "pipe closed".into(),
            state: ChannelState::Indeterminate,
            cause: ChannelCause::Transport,
        });
    let connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    let mut session = Session::open(connection, &config()).await.unwrap();

    let error = session
        .compile(CompileRequest::default())
        .await
        .unwrap_err();

    let CallError::Invalid(HostError::Channel { message, state, .. }) = error else {
        panic!("a failed shutdown keeps its channel state: {error:?}");
    };
    assert_eq!(state, ChannelState::Indeterminate);
    assert!(
        message.ends_with("; orderly shutdown also failed: pipe closed"),
        "{message}"
    );
}

// Call gating and request numbering, ported from the daemon's session tests.
// Each guest is held to `ExpectedChecks` for the id "example".

mod gating {
    use super::{config, ok};
    use morphir_extension_sdk::protocol::{InitializeResult, methods};
    use morphir_extension_sdk::{
        BackendCapability, ExtensionCapabilities, ExtensionInfo, ExtensionType, FrontendCapability,
        GenerateRequest, GenerateResult, WorkspaceCapability,
    };
    use morphir_host::testing::{MemoryChannel, SentLog};
    use morphir_host::{CallError, ExpectedChecks, ExpectedExtension, JsonRpcConnection, Session};
    use serde::Serialize;
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

    fn initialization(info: ExtensionInfo) -> InitializeResult {
        InitializeResult {
            protocol_version: "0.1".into(),
            extension: info,
            capabilities: ExtensionCapabilities::default(),
        }
    }

    fn frontend_initialization(compile: bool) -> InitializeResult {
        let mut result = initialization(extension(vec![ExtensionType::Frontend]));
        result.capabilities.frontend = Some(FrontendCapability {
            compile,
            ..FrontendCapability::default()
        });
        result
    }

    fn backend_initialization(generate: bool) -> InitializeResult {
        let mut result = initialization(extension(vec![ExtensionType::Backend]));
        result.capabilities.backend = Some(BackendCapability {
            targets: vec!["avro".into()],
            ir_versions: vec!["3".into(), "4".into()],
            generate,
        });
        result
    }

    fn workspace_initialization(discover: bool, protocol_versions: Vec<&str>) -> InitializeResult {
        let mut result = initialization(extension(vec![ExtensionType::Workspace]));
        result.capabilities.workspace = Some(WorkspaceCapability {
            protocol_versions: protocol_versions
                .into_iter()
                .map(|version| morphir_workspace::Version::parse(version).unwrap())
                .collect(),
            discover,
        });
        result
    }

    fn workspace_discovery_request(protocol_version: &str) -> serde_json::Value {
        json!({
            "protocolVersion": protocol_version,
            "developmentRoot": {"entries": {}},
            "morphirHome": null,
            "systemConfig": null,
            "environment": {},
            "cliOverlay": {}
        })
    }

    fn generate_request(target: &str) -> GenerateRequest {
        GenerateRequest {
            ir: serde_json::Value::Null,
            target: target.into(),
            options: Default::default(),
        }
    }

    /// Open a session on a guest that answers `responses` in order, the first
    /// being `initialize`.
    async fn open(
        expected: ExpectedExtension,
        responses: Vec<serde_json::Value>,
    ) -> (Session, SentLog) {
        let channel = responses
            .into_iter()
            .enumerate()
            .fold(MemoryChannel::new(), |channel, (index, value)| {
                channel.respond(ok(index as u64 + 1, value))
            });
        let log = channel.log();
        let connection = JsonRpcConnection::new(channel, ExpectedChecks::new(expected));
        let session = Session::open(connection, &config())
            .await
            .unwrap_or_else(|error| panic!("initialization failed: {error}"));
        (session, log)
    }

    async fn open_example(responses: Vec<serde_json::Value>) -> (Session, SentLog) {
        open(ExpectedExtension::identified("example"), responses).await
    }

    fn value(result: InitializeResult) -> serde_json::Value {
        serde_json::to_value(result).unwrap()
    }

    #[tokio::test]
    async fn explicit_schema_v1_discovery_retains_legacy_generation() {
        let (mut session, log) = open(
            ExpectedExtension::legacy_discovered(extension(vec![ExtensionType::Backend])),
            vec![
                value(initialization(extension(vec![ExtensionType::Backend]))),
                json!({"success": true, "artifacts": [], "diagnostics": []}),
            ],
        )
        .await;

        let result = session
            .generate(generate_request("legacy"))
            .await
            .unwrap_or_else(|error| panic!("legacy generation should succeed: {error:?}"));

        assert!(result.success);
        assert_eq!(log.methods(), [methods::INITIALIZE, methods::GENERATE]);
    }

    #[tokio::test]
    async fn rejects_generate_when_the_backend_did_not_enable_it_without_sending() {
        let (mut session, log) = open_example(vec![value(backend_initialization(false))]).await;

        match session.generate(generate_request("avro")).await {
            Err(CallError::Rejected(error)) => {
                assert!(error.to_string().contains("does not support capability"));
                assert_eq!(log.methods(), [methods::INITIALIZE]);
            }
            Ok(_) => panic!("disabled generation must be rejected locally"),
            Err(error) => panic!("local rejection should keep the session ready: {error:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_workspace_discovery_when_it_was_not_enabled_without_sending() {
        for initialized in [
            workspace_initialization(false, vec!["0.1.0-draft.1"]),
            workspace_initialization(true, vec!["0.1.0-draft.2"]),
        ] {
            let (mut session, log) = open_example(vec![value(initialized)]).await;

            match session
                .call::<_, serde_json::Value>(
                    methods::WORKSPACE_DISCOVER,
                    workspace_discovery_request("0.1.0-draft.1"),
                )
                .await
            {
                Err(CallError::Rejected(error)) => {
                    assert!(error.to_string().contains("does not support capability"));
                    assert_eq!(log.methods(), [methods::INITIALIZE]);
                }
                Ok(_) => panic!("disabled workspace discovery must be rejected locally"),
                Err(error) => panic!("local rejection should keep the session ready: {error:?}"),
            }
        }
    }

    #[tokio::test]
    async fn rejects_an_unsupported_workspace_request_protocol_without_sending() {
        let (mut session, log) = open_example(vec![value(workspace_initialization(
            true,
            vec!["0.1.0-draft.1"],
        ))])
        .await;

        match session
            .call::<_, serde_json::Value>(
                methods::WORKSPACE_DISCOVER,
                workspace_discovery_request("0.1.0-draft.2"),
            )
            .await
        {
            Err(CallError::Rejected(error)) => {
                assert!(error.to_string().contains("does not support capability"));
                assert_eq!(log.methods(), [methods::INITIALIZE]);
            }
            Ok(_) => panic!("protocol 2 must be rejected locally"),
            Err(error) => panic!("local rejection should keep the session ready: {error:?}"),
        }
    }

    #[tokio::test]
    async fn permits_workspace_discovery_for_protocol_v1() {
        let (mut session, log) = open_example(vec![
            value(workspace_initialization(true, vec!["0.1.0-draft.1"])),
            json!({
                "status": "failure",
                "error": {
                    "code": "workspace.config.missing",
                    "message": "No workspace configuration was found",
                    "path": null
                }
            }),
        ])
        .await;

        let result = session
            .call::<_, serde_json::Value>(
                methods::WORKSPACE_DISCOVER,
                workspace_discovery_request("0.1.0-draft.1"),
            )
            .await
            .unwrap_or_else(|error| {
                panic!("enabled workspace discovery should be sent: {error:?}")
            });

        assert_eq!(result["status"], "failure");
        assert_eq!(
            log.methods(),
            [methods::INITIALIZE, methods::WORKSPACE_DISCOVER]
        );
    }

    #[tokio::test]
    async fn local_serialization_failure_preserves_the_ready_session() {
        struct InvalidParams;
        impl Serialize for InvalidParams {
            fn serialize<S>(&self, _: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                Err(serde::ser::Error::custom("cannot serialize request"))
            }
        }

        let (mut session, log) = open_example(vec![value(backend_initialization(true))]).await;

        match session
            .call::<_, serde_json::Value>(methods::GENERATE, InvalidParams)
            .await
        {
            Err(CallError::Rejected(error)) => {
                assert!(error.to_string().contains("cannot serialize request"));
                assert_eq!(session.negotiated().extension().id, "example");
                assert_eq!(log.methods(), [methods::INITIALIZE]);
            }
            Ok(_) => panic!("invalid parameters should not be sent"),
            Err(error) => panic!("a local error should keep the session ready: {error:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_exit_as_a_ready_request_without_sending() {
        let (mut session, log) =
            open_example(vec![value(backend_initialization(true)), json!({})]).await;

        match session
            .call::<_, serde_json::Value>(methods::EXIT, json!({}))
            .await
        {
            Err(CallError::Rejected(error)) => {
                assert!(error.to_string().contains("lifecycle method"));
                assert_eq!(log.methods(), [methods::INITIALIZE]);
            }
            Ok(_) => panic!("exit must not be sent as a request"),
            Err(error) => panic!("local rejection should keep the session ready: {error:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_compile_when_the_frontend_did_not_enable_it_without_sending() {
        let (mut session, log) = open_example(vec![
            value(frontend_initialization(false)),
            json!({"success": false, "diagnostics": [], "modules": []}),
        ])
        .await;

        match session
            .call::<_, serde_json::Value>(methods::COMPILE, json!({}))
            .await
        {
            Err(CallError::Rejected(error)) => {
                assert!(error.to_string().contains("does not support capability"));
                assert_eq!(log.methods(), [methods::INITIALIZE]);
            }
            Ok(_) => panic!("disabled compilation should be rejected"),
            Err(error) => panic!("local rejection should keep the session ready: {error:?}"),
        }
    }

    #[tokio::test]
    async fn a_session_numbers_requests_in_the_released_order() {
        let (mut session, log) = open_example(vec![
            value(backend_initialization(true)),
            serde_json::to_value(GenerateResult {
                success: true,
                artifacts: Vec::new(),
                diagnostics: Vec::new(),
            })
            .unwrap(),
            json!({}),
        ])
        .await;

        session
            .generate(GenerateRequest {
                ir: json!({}),
                target: "avro".into(),
                options: Default::default(),
            })
            .await
            .unwrap_or_else(|error| panic!("generate should succeed: {error:?}"));
        session
            .close()
            .await
            .unwrap_or_else(|error| panic!("shutdown failed: {error}"));

        assert_eq!(
            log.methods(),
            [
                methods::INITIALIZE,
                methods::GENERATE,
                methods::SHUTDOWN,
                methods::EXIT
            ]
        );
        assert_eq!(log.ids(), [1, 2, 3]);
    }
}

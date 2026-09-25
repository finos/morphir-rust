use morphir_extension_sdk::protocol::{ExtensionResponse, PeerInfo, PeerKind, RpcError, methods};
use morphir_extension_sdk::{CompileRequest, CompileResult};
use morphir_host::testing::{MemoryChannel, frontend_initialize_result};
use morphir_host::{BasicChecks, CallError, HostConfig, JsonRpcConnection, Session, compile_once};
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
        Err(CallError::Decode(_)) => {}
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

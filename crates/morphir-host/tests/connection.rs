use morphir_extension_sdk::protocol::{
    ExtensionResponse, InitializeParams, PeerInfo, PeerKind, RpcError, methods,
};
use morphir_host::testing::{MemoryChannel, frontend_initialize_result};
use morphir_host::{
    BasicChecks, CallError, ChannelError, ChannelState, GuestConnection, JsonRpcConnection,
};
use serde_json::json;

fn params() -> InitializeParams {
    InitializeParams {
        protocol_versions: vec!["0.1".into()],
        host: PeerInfo {
            kind: PeerKind::Unspecified,
            name: "test".into(),
            version: "1.0.0".into(),
        },
    }
}

fn ok(id: u64, value: impl serde::Serialize) -> ExtensionResponse {
    ExtensionResponse::success(id, value).unwrap()
}

#[tokio::test]
async fn open_call_close_runs_the_mep_lifecycle() {
    let channel = MemoryChannel::new()
        .respond(ok(1, frontend_initialize_result("guest")))
        .respond(ok(2, json!({"answer": 42})))
        .respond(ok(3, json!({})));
    let log = channel.log();
    let mut connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));

    let negotiated = connection.open(params()).await.unwrap();
    assert_eq!(negotiated.extension().id, "guest");
    let value = connection.call(methods::COMPILE, json!({})).await.unwrap();
    assert_eq!(value, json!({"answer": 42}));
    connection.close().await.unwrap();

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
async fn a_rejected_call_keeps_the_connection_usable() {
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
        .respond(ok(3, json!({"second": true})));
    let log = channel.log();
    let mut connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    connection.open(params()).await.unwrap();

    match connection.call(methods::COMPILE, json!({})).await {
        Err(CallError::Rejected(error)) => {
            assert_eq!(error.to_string(), "RPC error -32001: does not compile")
        }
        other => panic!("expected a rejection, got {other:?}"),
    }
    let value = connection.call(methods::COMPILE, json!({})).await.unwrap();
    assert_eq!(value, json!({"second": true}));
    assert_eq!(log.closes(), 0);
}

#[tokio::test]
async fn an_invalid_response_closes_the_channel_and_fails() {
    let channel = MemoryChannel::new()
        .respond(ok(1, frontend_initialize_result("guest")))
        .respond(ok(42, json!({})));
    let log = channel.log();
    let mut connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    connection.open(params()).await.unwrap();

    match connection.call(methods::COMPILE, json!({})).await {
        Err(CallError::Failed(error)) => assert_eq!(
            error.to_string(),
            "Extension response ID 42 did not match request ID 2"
        ),
        other => panic!("expected a failure, got {other:?}"),
    }
    assert_eq!(log.closes(), 1);
}

#[tokio::test]
async fn a_transport_failure_does_not_close_the_channel_again() {
    let channel = MemoryChannel::new()
        .respond(ok(1, frontend_initialize_result("guest")))
        .fail(ChannelError {
            message: "pipe closed".into(),
            state: ChannelState::Indeterminate,
        });
    let log = channel.log();
    let mut connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    connection.open(params()).await.unwrap();

    match connection.call(methods::COMPILE, json!({})).await {
        Err(CallError::Failed(morphir_host::HostError::Channel { message, state })) => {
            assert_eq!(message, "pipe closed");
            assert_eq!(state, ChannelState::Indeterminate);
        }
        other => panic!("expected a channel failure, got {other:?}"),
    }
    assert_eq!(log.closes(), 0);
}

#[tokio::test]
async fn a_failed_handshake_closes_the_channel() {
    let channel = MemoryChannel::new().respond(ok(1, frontend_initialize_result("impostor")));
    let log = channel.log();
    let mut connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));

    let error = connection.open(params()).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Extension identity changed during initialization: expected 'guest', initialized 'impostor'"
    );
    assert_eq!(log.closes(), 1);
}

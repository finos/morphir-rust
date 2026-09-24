use morphir_extension_sdk::protocol::{
    ExtensionResponse, InitializeParams, PeerInfo, PeerKind, RpcError, methods,
};
use morphir_host::testing::{MemoryChannel, frontend_initialize_result};
use morphir_host::{
    BasicChecks, CallError, ChannelCause, ChannelError, ChannelState, GuestConnection,
    JsonRpcConnection,
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
async fn an_invalid_response_aborts_the_channel_and_fails() {
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
    assert_eq!(log.aborts(), 1);
    assert_eq!(log.closes(), 0);
}

#[tokio::test]
async fn a_transport_failure_does_not_close_the_channel_again() {
    let channel = MemoryChannel::new()
        .respond(ok(1, frontend_initialize_result("guest")))
        .fail(ChannelError {
            message: "pipe closed".into(),
            state: ChannelState::Indeterminate,
            cause: ChannelCause::Transport,
        });
    let log = channel.log();
    let mut connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    connection.open(params()).await.unwrap();

    match connection.call(methods::COMPILE, json!({})).await {
        Err(CallError::Failed(morphir_host::HostError::Channel { message, state, .. })) => {
            assert_eq!(message, "pipe closed");
            assert_eq!(state, ChannelState::Indeterminate);
        }
        other => panic!("expected a channel failure, got {other:?}"),
    }
    assert_eq!(log.closes(), 0);
}

#[tokio::test]
async fn a_transport_failure_keeps_what_it_began_as() {
    let channel = MemoryChannel::new()
        .respond(ok(1, frontend_initialize_result("guest")))
        .fail(ChannelError {
            message: "IO error: broken pipe".into(),
            state: ChannelState::Stopped,
            cause: ChannelCause::Io,
        });
    let mut connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    connection.open(params()).await.unwrap();

    match connection.call(methods::COMPILE, json!({})).await {
        Err(CallError::Failed(morphir_host::HostError::Channel { cause, .. })) => {
            assert_eq!(cause, ChannelCause::Io);
        }
        other => panic!("expected a channel failure, got {other:?}"),
    }
}

#[tokio::test]
async fn a_failed_handshake_aborts_the_channel() {
    let channel = MemoryChannel::new().respond(ok(1, frontend_initialize_result("impostor")));
    let log = channel.log();
    let mut connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));

    connection.open(params()).await.unwrap_err();
    assert_eq!(log.aborts(), 1);
    assert_eq!(log.closes(), 0);
}

#[tokio::test]
async fn an_orderly_close_closes_rather_than_aborts() {
    let channel = MemoryChannel::new()
        .respond(ok(1, frontend_initialize_result("guest")))
        .respond(ok(2, json!({})))
        .respond(ok(3, json!({})));
    let log = channel.log();
    let mut connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    connection.open(params()).await.unwrap();
    connection.call(methods::COMPILE, json!({})).await.unwrap();
    connection.close().await.unwrap();
    assert_eq!(log.closes(), 1);
    assert_eq!(log.aborts(), 0);
}

#[tokio::test]
async fn a_boxed_channel_is_a_channel() {
    let channel: Box<dyn morphir_host::Channel> = Box::new(
        MemoryChannel::new()
            .respond(ok(1, frontend_initialize_result("guest")))
            .respond(ok(2, json!({}))),
    );
    let mut connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    connection.open(params()).await.unwrap();
    connection.close().await.unwrap();
}

#[tokio::test]
async fn a_boxed_channel_forwards_abort() {
    let inner = MemoryChannel::new().respond(ok(1, frontend_initialize_result("impostor")));
    let log = inner.log();
    let channel: Box<dyn morphir_host::Channel> = Box::new(inner);
    let mut connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));

    connection.open(params()).await.unwrap_err();
    assert_eq!(log.aborts(), 1);
    assert_eq!(log.closes(), 0);
}

#[tokio::test]
async fn close_after_a_transport_failure_does_not_touch_the_channel() {
    let channel = MemoryChannel::new()
        .respond(ok(1, frontend_initialize_result("guest")))
        .fail(ChannelError {
            message: "pipe closed".into(),
            state: ChannelState::Indeterminate,
            cause: ChannelCause::Transport,
        });
    let log = channel.log();
    let mut connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    connection.open(params()).await.unwrap();
    connection
        .call(methods::COMPILE, json!({}))
        .await
        .unwrap_err();
    let methods_before = log.methods();

    connection.close().await.unwrap();

    assert_eq!(log.closes(), 0);
    assert_eq!(log.methods(), methods_before);
}

#[tokio::test]
async fn a_call_after_a_failure_fails_without_sending() {
    let channel = MemoryChannel::new()
        .respond(ok(1, frontend_initialize_result("guest")))
        .respond(ok(42, json!({})));
    let log = channel.log();
    let mut connection = JsonRpcConnection::new(channel, BasicChecks::new("guest"));
    connection.open(params()).await.unwrap();
    connection
        .call(methods::COMPILE, json!({}))
        .await
        .unwrap_err();
    let methods_before = log.methods();

    match connection.call(methods::COMPILE, json!({})).await {
        Err(CallError::Failed(_)) => {}
        other => panic!("expected a failure, got {other:?}"),
    }
    assert_eq!(log.methods(), methods_before);
}

use morphir_extension_sdk::native::doc_fixtures::DocFrontend;
use morphir_extension_sdk::protocol::{PeerInfo, PeerKind};
use morphir_extension_sdk::{CompileRequest, NativeExtension};
use morphir_host::{ExpectedChecks, HostConfig, JsonRpcConnection, Session};
use morphir_host_native::NativeChannel;

fn config() -> HostConfig {
    HostConfig::new(PeerInfo {
        kind: PeerKind::Unspecified,
        name: "test".into(),
        version: "1.0.0".into(),
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_native_frontend_compiles_through_a_session() {
    let extension = NativeExtension::frontend_only(DocFrontend).unwrap();
    let channel = NativeChannel::new(&extension);
    let checks = ExpectedChecks::new(channel.expectation());
    let connection = JsonRpcConnection::new(channel, checks);
    let mut session = Session::open(connection, &config()).await.unwrap();
    assert_eq!(session.negotiated().extension().id, extension.info().id);
    let result = session.compile(CompileRequest::default()).await;
    session.close().await.unwrap();
    assert!(result.is_ok());
}

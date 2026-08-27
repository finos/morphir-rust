use super::*;
use crate::DaemonError;
use crate::extensions::ConnectedDaemonTransport;
use crate::extensions::protocol::{
    ExtensionRequest, ExtensionResponse, InitializeParams, InitializeResult, methods,
};
use async_trait::async_trait;
use morphir_extension_sdk::{ExtensionCapabilities, ExtensionInfo, ExtensionType};
use serde::Serialize;
use std::collections::VecDeque;
use std::mem::size_of;

struct FakeTransport {
    expected: ExpectedExtension,
    responses: VecDeque<std::result::Result<ExtensionResponse, TransportError>>,
    termination: TransportState,
}

#[async_trait]
impl MepTransport for FakeTransport {
    fn expected_extension(&self) -> ExpectedExtension {
        self.expected.clone()
    }

    async fn exchange(
        &mut self,
        _: ExtensionRequest,
    ) -> std::result::Result<ExtensionResponse, TransportError> {
        self.responses
            .pop_front()
            .expect("a response should be arranged")
    }

    async fn terminate(&mut self) -> std::result::Result<TransportState, TransportError> {
        Ok(self.termination)
    }
}

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

fn params() -> InitializeParams {
    InitializeParams {
        protocol_versions: vec!["0.1".into()],
        host: crate::extensions::protocol::PeerInfo {
            name: "test".into(),
            version: "1".into(),
        },
    }
}

fn transport(expected: ExpectedExtension, response: ExtensionResponse) -> FakeTransport {
    FakeTransport {
        expected,
        responses: VecDeque::from([Ok(response)]),
        termination: TransportState::Stopped,
    }
}

#[tokio::test]
async fn rejects_an_invalid_response_envelope_before_negotiation() {
    let mut response =
        ExtensionResponse::success(1, initialization(extension(vec![ExtensionType::Backend])))
            .unwrap();
    response.jsonrpc = "1.0".into();
    let failure = Session::loaded(transport(
        ExpectedExtension::identified("example"),
        response,
    ))
    .initialize(params())
    .await
    .err()
    .expect("the envelope should fail");
    assert!(matches!(failure, FailedSession::Stopped(_, _)));
    assert!(failure.error().to_string().contains("JSON-RPC version"));
}

#[tokio::test]
async fn rejects_capability_drift_from_discovery() {
    let discovered = extension(vec![ExtensionType::Backend]);
    let initialized = extension(vec![ExtensionType::Frontend]);
    let response = ExtensionResponse::success(1, initialization(initialized)).unwrap();
    let failure = Session::loaded(transport(
        ExpectedExtension::discovered(discovered),
        response,
    ))
    .initialize(params())
    .await
    .err()
    .expect("capability drift should fail");
    assert!(
        failure
            .error()
            .to_string()
            .contains("disagreed with discovery")
    );
}

#[tokio::test]
async fn rejects_duplicate_capability_kinds() {
    let response = ExtensionResponse::success(
        1,
        initialization(extension(vec![
            ExtensionType::Backend,
            ExtensionType::Backend,
        ])),
    )
    .unwrap();
    let failure = Session::loaded(transport(
        ExpectedExtension::identified("example"),
        response,
    ))
    .initialize(params())
    .await
    .err()
    .expect("duplicates should fail");
    assert!(
        failure
            .error()
            .to_string()
            .contains("repeated a capability")
    );
}

#[tokio::test]
async fn retains_an_indeterminate_state_after_an_uncertain_exchange_failure() {
    let transport = FakeTransport {
        expected: ExpectedExtension::identified("example"),
        responses: VecDeque::from([Err(TransportError::new(
            DaemonError::Extension("connection lost".into()),
            TransportState::Indeterminate,
        ))]),
        termination: TransportState::Indeterminate,
    };
    let failure = Session::loaded(transport)
        .initialize(params())
        .await
        .err()
        .expect("the exchange should fail");
    assert!(matches!(failure, FailedSession::Indeterminate(_, _)));
}

#[test]
fn transport_trait_remains_object_safe() {
    fn accepts(_: Box<dyn MepTransport>) {}
    let response = ExtensionResponse::success(1, serde_json::json!({})).unwrap();
    accepts(Box::new(transport(
        ExpectedExtension::identified("example"),
        response,
    )));
}

#[test]
fn failed_session_size_is_bounded_for_large_transports() {
    assert!(size_of::<FailedSession<ConnectedDaemonTransport>>() <= 128);
}

#[tokio::test]
async fn local_serialization_failure_preserves_the_ready_session() {
    struct InvalidParams;
    impl Serialize for InvalidParams {
        fn serialize<S>(&self, _: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(serde::ser::Error::custom("cannot serialize request"))
        }
    }

    let initialized =
        ExtensionResponse::success(1, initialization(extension(vec![ExtensionType::Backend])))
            .unwrap();
    let session = Session::loaded(transport(
        ExpectedExtension::identified("example"),
        initialized,
    ))
    .initialize(params())
    .await
    .unwrap_or_else(|failure| panic!("initialization failed: {}", failure.error()));

    match session
        .invoke::<serde_json::Value>(methods::GENERATE, InvalidParams)
        .await
    {
        InvokeOutcome::Rejected(session, error) => {
            assert!(error.to_string().contains("cannot serialize request"));
            assert_eq!(session.negotiated().extension().id, "example");
        }
        InvokeOutcome::Success(_, _) => panic!("invalid parameters should not be sent"),
        InvokeOutcome::Failed(failure) => {
            panic!("a local error should preserve Ready: {}", failure.error())
        }
    }
}

use super::*;
use crate::DaemonError;
use crate::extensions::ConnectedDaemonTransport;
use crate::extensions::protocol::{
    ExtensionRequest, ExtensionResponse, InitializeParams, InitializeResult, methods,
};
use async_trait::async_trait;
use morphir_extension_sdk::{
    CompileResult, ExtensionCapabilities, ExtensionInfo, ExtensionType, FrontendCapability,
};
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

fn frontend_initialization(compile: bool) -> InitializeResult {
    let mut result = initialization(extension(vec![ExtensionType::Frontend]));
    result.capabilities.frontend = Some(FrontendCapability {
        compile,
        ..FrontendCapability::default()
    });
    result
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

fn compile_params(ir_version: &str) -> serde_json::Value {
    serde_json::json!({
        "languageId": "elm",
        "documents": [],
        "package": {"name": "example/package", "exposedModules": []},
        "dependencies": [],
        "options": {"typesOnly": false, "irVersion": ir_version}
    })
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
async fn rejects_declared_frontend_without_frontend_capabilities() {
    let response =
        ExtensionResponse::success(1, initialization(extension(vec![ExtensionType::Frontend])))
            .unwrap();
    let failure = Session::loaded(transport(
        ExpectedExtension::identified("example"),
        response,
    ))
    .initialize(params())
    .await
    .err()
    .expect("missing frontend capabilities should fail");

    assert!(
        failure
            .error()
            .to_string()
            .contains("declared Frontend without frontend capabilities")
    );
}

#[tokio::test]
async fn rejects_frontend_capabilities_without_declared_frontend() {
    let mut result = initialization(extension(vec![ExtensionType::Backend]));
    result.capabilities.frontend = Some(FrontendCapability::default());
    let response = ExtensionResponse::success(1, result).unwrap();
    let failure = Session::loaded(transport(
        ExpectedExtension::identified("example"),
        response,
    ))
    .initialize(params())
    .await
    .err()
    .expect("undeclared frontend capabilities should fail");

    assert!(
        failure
            .error()
            .to_string()
            .contains("frontend capabilities without declaring Frontend")
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

#[tokio::test]
async fn rejects_exit_as_a_ready_request_without_sending() {
    let initialized =
        ExtensionResponse::success(1, initialization(extension(vec![ExtensionType::Backend])))
            .unwrap();
    let exit_response = ExtensionResponse::success(2, serde_json::json!({})).unwrap();
    let transport = FakeTransport {
        expected: ExpectedExtension::identified("example"),
        responses: VecDeque::from([Ok(initialized), Ok(exit_response)]),
        termination: TransportState::Stopped,
    };
    let session = Session::loaded(transport)
        .initialize(params())
        .await
        .unwrap_or_else(|failure| panic!("initialization failed: {}", failure.error()));

    match session
        .invoke::<serde_json::Value>(methods::EXIT, serde_json::json!({}))
        .await
    {
        InvokeOutcome::Rejected(session, error) => {
            assert!(error.to_string().contains("lifecycle method"));
            assert_eq!(session.transport_internal().responses.len(), 1);
        }
        InvokeOutcome::Success(_, _) => panic!("exit must not be sent as a request"),
        InvokeOutcome::Failed(failure) => {
            panic!("local rejection should preserve Ready: {}", failure.error())
        }
    }
}

#[tokio::test]
async fn rejects_compile_when_the_frontend_did_not_enable_it_without_sending() {
    let initialized = ExtensionResponse::success(1, frontend_initialization(false)).unwrap();
    let compile_response = ExtensionResponse::success(
        2,
        serde_json::json!({
            "success": false,
            "diagnostics": [],
            "modules": []
        }),
    )
    .unwrap();
    let transport = FakeTransport {
        expected: ExpectedExtension::identified("example"),
        responses: VecDeque::from([Ok(initialized), Ok(compile_response)]),
        termination: TransportState::Stopped,
    };
    let session = Session::loaded(transport)
        .initialize(params())
        .await
        .unwrap_or_else(|failure| panic!("initialization failed: {}", failure.error()));

    match session
        .invoke::<serde_json::Value>(methods::COMPILE, serde_json::json!({}))
        .await
    {
        InvokeOutcome::Rejected(session, error) => {
            assert!(error.to_string().contains("does not support capability"));
            assert_eq!(session.transport_internal().responses.len(), 1);
        }
        InvokeOutcome::Success(_, _) => panic!("disabled compilation should be rejected"),
        InvokeOutcome::Failed(failure) => {
            panic!("local rejection should preserve Ready: {}", failure.error())
        }
    }
}

#[tokio::test]
async fn permits_compile_when_the_frontend_enabled_it() {
    let initialized = ExtensionResponse::success(1, frontend_initialization(true)).unwrap();
    let compile_response = ExtensionResponse::success(
        2,
        serde_json::json!({
            "success": false,
            "diagnostics": [],
            "modules": []
        }),
    )
    .unwrap();
    let transport = FakeTransport {
        expected: ExpectedExtension::identified("example"),
        responses: VecDeque::from([Ok(initialized), Ok(compile_response)]),
        termination: TransportState::Stopped,
    };
    let session = Session::loaded(transport)
        .initialize(params())
        .await
        .unwrap_or_else(|failure| panic!("initialization failed: {}", failure.error()));

    match session
        .invoke::<serde_json::Value>(methods::COMPILE, serde_json::json!({}))
        .await
    {
        InvokeOutcome::Success(session, result) => {
            assert_eq!(result["success"], false);
            assert!(session.transport_internal().responses.is_empty());
        }
        InvokeOutcome::Rejected(_, error) => panic!("compile should be sent: {error}"),
        InvokeOutcome::Failed(failure) => panic!("compile should succeed: {}", failure.error()),
    }
}

#[tokio::test]
async fn rejects_successful_compile_result_without_ir_version() {
    let initialized = ExtensionResponse::success(1, frontend_initialization(true)).unwrap();
    let compile_response = ExtensionResponse::success(
        2,
        serde_json::json!({
            "success": true,
            "ir": {},
            "diagnostics": [],
            "modules": []
        }),
    )
    .unwrap();
    let transport = FakeTransport {
        expected: ExpectedExtension::identified("example"),
        responses: VecDeque::from([Ok(initialized), Ok(compile_response)]),
        termination: TransportState::Stopped,
    };
    let session = Session::loaded(transport)
        .initialize(params())
        .await
        .unwrap_or_else(|failure| panic!("initialization failed: {}", failure.error()));

    match session
        .invoke::<CompileResult>(methods::COMPILE, compile_params("3"))
        .await
    {
        InvokeOutcome::Failed(failure) => {
            assert!(failure.error().to_string().contains("missing irVersion"));
        }
        InvokeOutcome::Success(_, _) => panic!("malformed success should fail the session"),
        InvokeOutcome::Rejected(_, error) => {
            panic!("malformed success is not an RPC error: {error}")
        }
    }
}

#[tokio::test]
async fn rejects_successful_compile_result_without_ir_for_raw_callers() {
    let initialized = ExtensionResponse::success(1, frontend_initialization(true)).unwrap();
    let compile_response = ExtensionResponse::success(
        2,
        serde_json::json!({
            "success": true,
            "irVersion": "3",
            "diagnostics": [],
            "modules": []
        }),
    )
    .unwrap();
    let transport = FakeTransport {
        expected: ExpectedExtension::identified("example"),
        responses: VecDeque::from([Ok(initialized), Ok(compile_response)]),
        termination: TransportState::Stopped,
    };
    let session = Session::loaded(transport)
        .initialize(params())
        .await
        .unwrap_or_else(|failure| panic!("initialization failed: {}", failure.error()));

    match session
        .invoke::<serde_json::Value>(methods::COMPILE, serde_json::json!({}))
        .await
    {
        InvokeOutcome::Failed(failure) => {
            assert!(failure.error().to_string().contains("missing ir"));
        }
        InvokeOutcome::Success(_, _) => panic!("malformed success should fail the session"),
        InvokeOutcome::Rejected(_, error) => {
            panic!("malformed success is not an RPC error: {error}")
        }
    }
}

#[tokio::test]
async fn accepts_successful_compile_result_with_ir_version_and_ir() {
    let initialized = ExtensionResponse::success(1, frontend_initialization(true)).unwrap();
    let compile_response = ExtensionResponse::success(
        2,
        serde_json::json!({
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
    .unwrap();
    let transport = FakeTransport {
        expected: ExpectedExtension::identified("example"),
        responses: VecDeque::from([Ok(initialized), Ok(compile_response)]),
        termination: TransportState::Stopped,
    };
    let session = Session::loaded(transport)
        .initialize(params())
        .await
        .unwrap_or_else(|failure| panic!("initialization failed: {}", failure.error()));

    match session
        .invoke::<CompileResult>(methods::COMPILE, compile_params("3"))
        .await
    {
        InvokeOutcome::Success(_, result) => {
            assert!(result.success);
            assert_eq!(result.ir_version.as_deref(), Some("3"));
            assert!(result.ir.is_some());
        }
        InvokeOutcome::Rejected(_, error) => panic!("valid success was rejected: {error}"),
        InvokeOutcome::Failed(failure) => {
            panic!("valid success failed the session: {}", failure.error())
        }
    }
}

#[tokio::test]
async fn rejects_a_successful_compile_result_for_another_requested_ir_version() {
    let initialized = ExtensionResponse::success(1, frontend_initialization(true)).unwrap();
    let compile_response = ExtensionResponse::success(
        2,
        serde_json::json!({
            "success": true,
            "irVersion": "4.0.0",
            "ir": {"Library": {}},
            "diagnostics": [],
            "modules": []
        }),
    )
    .unwrap();
    let transport = FakeTransport {
        expected: ExpectedExtension::identified("example"),
        responses: VecDeque::from([Ok(initialized), Ok(compile_response)]),
        termination: TransportState::Stopped,
    };
    let session = Session::loaded(transport)
        .initialize(params())
        .await
        .unwrap_or_else(|failure| panic!("initialization failed: {}", failure.error()));

    match session
        .invoke::<CompileResult>(methods::COMPILE, compile_params("3"))
        .await
    {
        InvokeOutcome::Failed(failure) => assert!(
            failure
                .error()
                .to_string()
                .contains("did not match requested irVersion")
        ),
        InvokeOutcome::Success(_, _) => panic!("mismatched result version should fail the session"),
        InvokeOutcome::Rejected(_, error) => panic!("mismatch is not an RPC error: {error}"),
    }
}

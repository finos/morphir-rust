use super::*;
use crate::DaemonError;
use crate::extensions::ConnectedDaemonTransport;
use crate::extensions::protocol::{
    ExtensionRequest, ExtensionResponse, InitializeParams, InitializeResult, methods,
};
use async_trait::async_trait;
use morphir_extension_sdk::{
    BackendCapability, CompileResult, ExtensionCapabilities, ExtensionInfo, ExtensionType,
    FrontendCapability, GenerateRequest, GenerateResult,
};
use serde::Serialize;
use std::collections::VecDeque;
use std::mem::size_of;

struct FakeTransport {
    expected: ExpectedExtension,
    responses: VecDeque<std::result::Result<ExtensionResponse, TransportError>>,
    termination: TransportState,
}

struct ScriptedTransport {
    expected: ExpectedExtension,
    responses: VecDeque<std::result::Result<ExtensionResponse, TransportError>>,
    requests: Vec<ExtensionRequest>,
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

#[async_trait]
impl MepTransport for ScriptedTransport {
    fn expected_extension(&self) -> ExpectedExtension {
        self.expected.clone()
    }

    async fn exchange(
        &mut self,
        request: ExtensionRequest,
    ) -> std::result::Result<ExtensionResponse, TransportError> {
        self.requests.push(request);
        self.responses
            .pop_front()
            .expect("a response should be arranged")
    }

    async fn terminate(&mut self) -> std::result::Result<TransportState, TransportError> {
        Ok(TransportState::Stopped)
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

fn backend_initialization(generate: bool) -> InitializeResult {
    let mut result = initialization(extension(vec![ExtensionType::Backend]));
    result.capabilities.backend = Some(BackendCapability {
        targets: vec!["avro".into()],
        ir_versions: vec!["3".into(), "4".into()],
        generate,
    });
    result
}

fn scripted_transport(responses: impl IntoIterator<Item = ExtensionResponse>) -> ScriptedTransport {
    ScriptedTransport {
        expected: ExpectedExtension::identified("example"),
        responses: responses.into_iter().map(Ok).collect(),
        requests: Vec::new(),
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
            ExtensionType::Validator,
            ExtensionType::Validator,
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
    let mut result = initialization(extension(vec![ExtensionType::Validator]));
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
async fn rejects_declared_backend_without_backend_capabilities() {
    let response =
        ExtensionResponse::success(1, initialization(extension(vec![ExtensionType::Backend])))
            .unwrap();
    let failure = Session::loaded(transport(
        ExpectedExtension::identified("example"),
        response,
    ))
    .initialize(params())
    .await
    .err()
    .expect("missing backend capabilities should fail");

    assert!(
        failure
            .error()
            .to_string()
            .contains("declared Backend without backend capabilities")
    );
}

#[tokio::test]
async fn rejects_backend_capabilities_without_declared_backend() {
    let mut result = initialization(extension(vec![ExtensionType::Validator]));
    result.capabilities.backend = Some(BackendCapability {
        targets: vec!["avro".into()],
        ir_versions: vec!["3".into(), "4".into()],
        generate: true,
    });
    let response = ExtensionResponse::success(1, result).unwrap();
    let failure = Session::loaded(transport(
        ExpectedExtension::identified("example"),
        response,
    ))
    .initialize(params())
    .await
    .err()
    .expect("undeclared backend capabilities should fail");

    assert!(
        failure
            .error()
            .to_string()
            .contains("backend capabilities without declaring Backend")
    );
}

#[tokio::test]
async fn rejects_each_backend_metadata_drift_before_generate() {
    let locked_backend = BackendCapability {
        targets: vec!["avro".into(), "json-schema".into()],
        ir_versions: vec!["3".into(), "4".into()],
        generate: true,
    };
    let cases = [
        ("missing backend", None),
        (
            "changed target",
            Some(BackendCapability {
                targets: vec!["avro".into(), "protobuf".into()],
                ..locked_backend.clone()
            }),
        ),
        (
            "changed target order",
            Some(BackendCapability {
                targets: vec!["json-schema".into(), "avro".into()],
                ..locked_backend.clone()
            }),
        ),
        (
            "changed IR version",
            Some(BackendCapability {
                ir_versions: vec!["3".into(), "5".into()],
                ..locked_backend.clone()
            }),
        ),
        (
            "changed IR version order",
            Some(BackendCapability {
                ir_versions: vec!["4".into(), "3".into()],
                ..locked_backend.clone()
            }),
        ),
        (
            "changed generate flag",
            Some(BackendCapability {
                generate: false,
                ..locked_backend.clone()
            }),
        ),
    ];

    for (case, initialized_backend) in cases {
        let locked = ExtensionCapabilities {
            backend: Some(locked_backend.clone()),
            ..ExtensionCapabilities::default()
        };
        let mut initialized = initialization(extension(vec![ExtensionType::Backend]));
        initialized.capabilities.backend = initialized_backend;
        let response = ExtensionResponse::success(1, initialized).unwrap();
        let transport = ScriptedTransport {
            expected: ExpectedExtension::discovered_with_capabilities(
                extension(vec![ExtensionType::Backend]),
                locked,
            ),
            responses: [Ok(response)].into(),
            requests: Vec::new(),
        };

        let failure = Session::loaded(transport)
            .initialize(params())
            .await
            .err()
            .unwrap_or_else(|| panic!("{case} should fail initialization"));

        assert!(
            failure
                .error()
                .to_string()
                .contains("backend capabilities disagreed with discovery"),
            "{case}: {}",
            failure.error()
        );
        match failure {
            FailedSession::Stopped(session, _) => {
                assert_eq!(session.transport_internal().requests.len(), 1, "{case}");
                assert_eq!(
                    session.transport_internal().requests[0].method,
                    methods::INITIALIZE,
                    "{case}"
                );
            }
            FailedSession::Indeterminate(_, error) => {
                panic!("{case}: fake transport abort should prove stopped: {error}")
            }
        }
    }
}

#[tokio::test]
async fn rejects_each_non_backend_locked_capability_drift() {
    let locked = ExtensionCapabilities {
        frontend: Some(FrontendCapability {
            compile: true,
            ..FrontendCapability::default()
        }),
        streaming: true,
        extra: [("vendor.feature".to_owned(), serde_json::json!("locked"))]
            .into_iter()
            .collect(),
        ..ExtensionCapabilities::default()
    };
    let mut changed_frontend = locked.clone();
    changed_frontend.frontend.as_mut().unwrap().compile = false;
    let mut changed_streaming = locked.clone();
    changed_streaming.streaming = false;
    let mut changed_extra = locked.clone();
    changed_extra
        .extra
        .insert("vendor.feature".to_owned(), serde_json::json!("changed"));

    for (case, capabilities) in [
        ("frontend", changed_frontend),
        ("streaming", changed_streaming),
        ("extension-specific", changed_extra),
    ] {
        let mut initialized = initialization(extension(vec![ExtensionType::Frontend]));
        initialized.capabilities = capabilities;
        let response = ExtensionResponse::success(1, initialized).unwrap();
        let transport = ScriptedTransport {
            expected: ExpectedExtension::discovered_with_capabilities(
                extension(vec![ExtensionType::Frontend]),
                locked.clone(),
            ),
            responses: [Ok(response)].into(),
            requests: Vec::new(),
        };

        let failure = Session::loaded(transport)
            .initialize(params())
            .await
            .err()
            .unwrap_or_else(|| panic!("{case} capability drift should fail initialization"));

        assert!(
            failure
                .error()
                .to_string()
                .contains("capabilities disagreed with discovery"),
            "{case}: {}",
            failure.error()
        );
    }
}

#[tokio::test]
async fn backend_only_lock_accepts_unpersisted_capabilities() {
    let backend = BackendCapability {
        targets: vec!["avro".into()],
        ir_versions: vec!["4".into()],
        generate: true,
    };
    let mut initialized = initialization(extension(vec![
        ExtensionType::Backend,
        ExtensionType::Frontend,
    ]));
    initialized.capabilities = ExtensionCapabilities {
        backend: Some(backend.clone()),
        frontend: Some(FrontendCapability {
            compile: true,
            ..FrontendCapability::default()
        }),
        streaming: true,
        extra: [("vendor.feature".to_owned(), serde_json::json!("guest"))]
            .into_iter()
            .collect(),
        ..ExtensionCapabilities::default()
    };
    let response = ExtensionResponse::success(1, initialized).unwrap();
    let transport = ScriptedTransport {
        expected: ExpectedExtension::discovered_with_backend_capability(
            extension(vec![ExtensionType::Backend, ExtensionType::Frontend]),
            backend,
        ),
        responses: [Ok(response)].into(),
        requests: Vec::new(),
    };

    let session = match Session::loaded(transport).initialize(params()).await {
        Ok(session) => session,
        Err(failure) => panic!(
            "capabilities absent from installed metadata must remain negotiable: {}",
            failure.error()
        ),
    };

    assert!(session.negotiated().capabilities.streaming);
    assert!(session.negotiated().capabilities.frontend.is_some());
}

#[tokio::test]
async fn backend_only_lock_rejects_backend_drift() {
    let locked = BackendCapability {
        targets: vec!["avro".into()],
        ir_versions: vec!["4".into()],
        generate: true,
    };
    let mut initialized = backend_initialization(true);
    initialized.capabilities.backend.as_mut().unwrap().targets = vec!["json-schema".into()];
    let response = ExtensionResponse::success(1, initialized).unwrap();
    let transport = ScriptedTransport {
        expected: ExpectedExtension::discovered_with_backend_capability(
            extension(vec![ExtensionType::Backend]),
            locked,
        ),
        responses: [Ok(response)].into(),
        requests: Vec::new(),
    };

    let failure = match Session::loaded(transport).initialize(params()).await {
        Ok(_) => panic!("the persisted backend contract must remain exact"),
        Err(failure) => failure,
    };

    assert!(
        failure
            .error()
            .to_string()
            .contains("backend capabilities disagreed with discovery")
    );
}

#[tokio::test]
async fn discovered_backend_without_locked_metadata_accepts_valid_initialization() {
    let initialized = ExtensionResponse::success(1, backend_initialization(true)).unwrap();
    let transport = ScriptedTransport {
        expected: ExpectedExtension::discovered(extension(vec![ExtensionType::Backend])),
        responses: [Ok(initialized)].into(),
        requests: Vec::new(),
    };

    let session = Session::loaded(transport)
        .initialize(params())
        .await
        .unwrap_or_else(|failure| panic!("initialization failed: {}", failure.error()));

    assert_eq!(session.transport_internal().requests.len(), 1);
    assert_eq!(
        session.transport_internal().requests[0].method,
        methods::INITIALIZE
    );
}

#[tokio::test]
async fn ordinary_discovery_rejects_a_backend_without_typed_capabilities() {
    let initialized =
        ExtensionResponse::success(1, initialization(extension(vec![ExtensionType::Backend])))
            .unwrap();
    let transport = ScriptedTransport {
        expected: ExpectedExtension::discovered(extension(vec![ExtensionType::Backend])),
        responses: [Ok(initialized)].into(),
        requests: Vec::new(),
    };
    let failure = match Session::loaded(transport).initialize(params()).await {
        Ok(_) => panic!("ordinary discovery must not enable schema-v1 compatibility"),
        Err(failure) => failure,
    };

    assert!(
        failure
            .error()
            .to_string()
            .contains("declared Backend without backend capabilities")
    );
}

#[tokio::test]
async fn explicit_schema_v1_discovery_retains_legacy_generation() {
    let initialized =
        ExtensionResponse::success(1, initialization(extension(vec![ExtensionType::Backend])))
            .unwrap();
    let generated = ExtensionResponse::success(
        2,
        serde_json::json!({"success": true, "artifacts": [], "diagnostics": []}),
    )
    .unwrap();
    let transport = ScriptedTransport {
        expected: ExpectedExtension::legacy_discovered(extension(vec![ExtensionType::Backend])),
        responses: [Ok(initialized), Ok(generated)].into(),
        requests: Vec::new(),
    };
    let session = Session::loaded(transport)
        .initialize(params())
        .await
        .unwrap_or_else(|failure| panic!("legacy initialization failed: {}", failure.error()));

    match session
        .invoke::<GenerateResult>(methods::GENERATE, GenerateRequest::default())
        .await
    {
        InvokeOutcome::Success(session, result) => {
            assert!(result.success);
            assert_eq!(session.transport_internal().requests.len(), 2);
        }
        InvokeOutcome::Rejected(_, error) => {
            panic!("legacy generation should remain available: {error}")
        }
        InvokeOutcome::Failed(failure) => {
            panic!("legacy generation should succeed: {}", failure.error())
        }
    }
}

#[tokio::test]
async fn rejects_generate_when_the_backend_did_not_enable_it_without_sending() {
    let initialized = ExtensionResponse::success(1, backend_initialization(false)).unwrap();
    let session = Session::loaded(scripted_transport([initialized]))
        .initialize(params())
        .await
        .unwrap_or_else(|failure| panic!("initialization failed: {}", failure.error()));

    match session
        .invoke::<GenerateResult>(methods::GENERATE, GenerateRequest::default())
        .await
    {
        InvokeOutcome::Rejected(session, error) => {
            assert!(error.to_string().contains("does not support capability"));
            assert_eq!(session.transport_internal().requests.len(), 1);
            assert_eq!(
                session.transport_internal().requests[0].method,
                methods::INITIALIZE
            );
        }
        InvokeOutcome::Success(_, _) => panic!("disabled generation must be rejected locally"),
        InvokeOutcome::Failed(failure) => {
            panic!("local rejection should preserve Ready: {}", failure.error())
        }
    }
}

#[tokio::test]
async fn permits_generate_when_the_backend_enabled_it() {
    let initialized = ExtensionResponse::success(1, backend_initialization(true)).unwrap();
    let generated = ExtensionResponse::success(
        2,
        serde_json::json!({"success": true, "artifacts": [], "diagnostics": []}),
    )
    .unwrap();
    let session = Session::loaded(scripted_transport([initialized, generated]))
        .initialize(params())
        .await
        .unwrap_or_else(|failure| panic!("initialization failed: {}", failure.error()));

    match session
        .invoke::<GenerateResult>(methods::GENERATE, GenerateRequest::default())
        .await
    {
        InvokeOutcome::Success(session, result) => {
            assert!(result.success);
            assert_eq!(session.transport_internal().requests.len(), 2);
            assert_eq!(
                session.transport_internal().requests[1].method,
                methods::GENERATE
            );
        }
        InvokeOutcome::Rejected(_, error) => panic!("enabled generation should be sent: {error}"),
        InvokeOutcome::Failed(failure) => panic!("generation should succeed: {}", failure.error()),
    }
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

    let initialized = ExtensionResponse::success(1, backend_initialization(true)).unwrap();
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
    let initialized = ExtensionResponse::success(1, backend_initialization(true)).unwrap();
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

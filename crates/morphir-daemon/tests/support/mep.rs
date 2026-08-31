use morphir_daemon::extensions::{
    ExtensionSession, ExtensionSessionState, InvokeOutcome, ProcessLaunch, Ready, Session,
    SpawnedProcessSession, SpawnedProcessTransport, protocol::methods,
};
use morphir_extension_sdk::{
    CompileRequest, CompileResult, DiagnosticSeverity, ExtensionType, GenerateRequest,
    GenerateResult, ValidateRequest,
    protocol::{InitializeParams, PeerInfo, error_codes},
};
use serde_json::json;

#[allow(dead_code)]
pub async fn assert_frontend_typestate_conformance(
    launch: ProcessLaunch,
    valid_request: CompileRequest,
    malformed_request: CompileRequest,
) {
    let valid_session = initialize_frontend(launch.clone()).await;
    let valid_session = match valid_session
        .invoke::<CompileResult>(methods::COMPILE, valid_request)
        .await
    {
        InvokeOutcome::Success(session, result) => {
            assert!(result.success, "valid Elm should compile successfully");
            assert_eq!(result.ir_version.as_deref(), Some("3"));
            assert!(result.ir.is_some(), "a successful compile should return IR");
            assert!(
                result.modules.iter().any(|module| module == "Example"),
                "a successful compile should report the Example module"
            );
            session
        }
        InvokeOutcome::Rejected(_, error) => panic!("valid Elm was rejected: {error}"),
        InvokeOutcome::Failed(failure) => {
            panic!("valid Elm failed the MEP session: {}", failure.error())
        }
    };
    shutdown_frontend(valid_session).await;

    let malformed_session = initialize_frontend(launch).await;
    let malformed_session = match malformed_session
        .invoke::<CompileResult>(methods::COMPILE, malformed_request)
        .await
    {
        InvokeOutcome::Success(session, result) => {
            assert!(!result.success, "malformed Elm should not compile");
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error),
                "malformed Elm should return an error diagnostic"
            );
            session
        }
        InvokeOutcome::Rejected(_, error) => panic!("malformed Elm was rejected: {error}"),
        InvokeOutcome::Failed(failure) => {
            panic!("malformed Elm failed the MEP session: {}", failure.error())
        }
    };
    shutdown_frontend(malformed_session).await;
}

async fn initialize_frontend(launch: ProcessLaunch) -> Session<SpawnedProcessTransport, Ready> {
    let session = SpawnedProcessSession::spawn_typestate(launch)
        .await
        .expect("the host should start the frontend extension");
    let session = session
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "morphir-conformance".into(),
                version: "0.1.0".into(),
            },
        })
        .await
        .unwrap_or_else(|failure| panic!("MEP negotiation failed: {}", failure.error()));

    assert_eq!(session.negotiated().protocol_version(), "0.1");
    assert_eq!(session.negotiated().extension().id, "morphir-elm");
    assert!(
        session
            .negotiated()
            .extension()
            .types
            .contains(&ExtensionType::Frontend),
        "morphir-elm should declare the frontend capability"
    );
    let frontend = session
        .negotiated()
        .capabilities()
        .frontend
        .as_ref()
        .expect("morphir-elm should advertise frontend details");
    assert!(
        frontend.compile,
        "morphir-elm should accept compile requests"
    );
    assert!(
        frontend
            .languages
            .iter()
            .any(|language| language.id == "elm"),
        "morphir-elm should advertise Elm"
    );
    assert!(
        frontend.ir_versions.iter().any(|version| version == "3"),
        "morphir-elm should advertise Morphir IR 3"
    );

    session
}

async fn shutdown_frontend(session: Session<SpawnedProcessTransport, Ready>) {
    let mut session = session
        .shutdown()
        .await
        .unwrap_or_else(|failure| panic!("MEP shutdown failed: {}", failure.error()));
    assert!(
        !session
            .process_is_running()
            .expect("process status should be readable"),
        "the frontend process should stop after shutdown"
    );
    assert!(
        session
            .process_stdout_is_exhausted()
            .await
            .expect("frontend stdout should be readable after shutdown"),
        "frontend stdout should contain only the framed protocol responses"
    );
}

#[allow(dead_code)]
pub async fn assert_backend_typestate_conformance<T>(
    session: morphir_daemon::extensions::Session<T, morphir_daemon::extensions::Loaded>,
    target: &str,
    valid_ir: serde_json::Value,
    invalid_ir: serde_json::Value,
) -> morphir_daemon::extensions::Session<T, morphir_daemon::extensions::Stopped>
where
    T: morphir_daemon::extensions::MepTransport,
{
    let session = initialize_backend(session).await;
    let session = generate_successfully(session, target, valid_ir).await;
    let session = match session
        .invoke::<GenerateResult>(
            methods::GENERATE,
            GenerateRequest {
                ir: invalid_ir,
                target: target.into(),
                options: Default::default(),
            },
        )
        .await
    {
        InvokeOutcome::Success(session, generated) => {
            assert!(!generated.success);
            assert!(!generated.diagnostics.is_empty());
            session
        }
        InvokeOutcome::Rejected(_, error) => panic!("generation was rejected: {error}"),
        InvokeOutcome::Failed(failure) => panic!("generation failed: {}", failure.error()),
    };

    shutdown_backend(session).await
}

#[allow(dead_code)]
pub async fn assert_backend_conformance<T, const IR_VERSION_COUNT: usize>(
    session: morphir_daemon::extensions::Session<T, morphir_daemon::extensions::Loaded>,
    expected_target: &str,
    expected_ir_versions: [&str; IR_VERSION_COUNT],
) -> morphir_daemon::extensions::Session<T, morphir_daemon::extensions::Stopped>
where
    T: morphir_daemon::extensions::MepTransport,
{
    let session = initialize_backend(session).await;
    let backend = session
        .negotiated()
        .capabilities()
        .backend
        .as_ref()
        .expect("the backend capability should be negotiated");
    assert_eq!(backend.targets, [expected_target]);
    assert_eq!(backend.ir_versions, expected_ir_versions);
    assert!(backend.generate, "the backend should support generation");

    shutdown_backend(
        generate_successfully(session, expected_target, supported_v4_distribution()).await,
    )
    .await
}

async fn initialize_backend<T>(
    session: morphir_daemon::extensions::Session<T, morphir_daemon::extensions::Loaded>,
) -> morphir_daemon::extensions::Session<T, morphir_daemon::extensions::Ready>
where
    T: morphir_daemon::extensions::MepTransport,
{
    let session = session
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "morphir-conformance".into(),
                version: "0.1.0".into(),
            },
        })
        .await
        .unwrap_or_else(|failure| panic!("MEP negotiation failed: {}", failure.error()));
    assert_eq!(session.negotiated().protocol_version(), "0.1");
    assert!(
        session
            .negotiated()
            .extension()
            .types
            .contains(&ExtensionType::Backend)
    );
    session
}

async fn generate_successfully<T>(
    session: morphir_daemon::extensions::Session<T, morphir_daemon::extensions::Ready>,
    target: &str,
    ir: serde_json::Value,
) -> morphir_daemon::extensions::Session<T, morphir_daemon::extensions::Ready>
where
    T: morphir_daemon::extensions::MepTransport,
{
    match session
        .invoke::<GenerateResult>(
            methods::GENERATE,
            GenerateRequest {
                ir,
                target: target.into(),
                options: Default::default(),
            },
        )
        .await
    {
        InvokeOutcome::Success(session, generated) => {
            assert!(generated.success);
            assert!(!generated.artifacts.is_empty());
            session
        }
        InvokeOutcome::Rejected(_, error) => panic!("generation was rejected: {error}"),
        InvokeOutcome::Failed(failure) => panic!("generation failed: {}", failure.error()),
    }
}

async fn shutdown_backend<T>(
    session: morphir_daemon::extensions::Session<T, morphir_daemon::extensions::Ready>,
) -> morphir_daemon::extensions::Session<T, morphir_daemon::extensions::Stopped>
where
    T: morphir_daemon::extensions::MepTransport,
{
    session.shutdown().await.unwrap_or_else(|failure| {
        panic!(
            "MEP shutdown or transport termination failed: {}",
            failure.error()
        )
    })
}

fn supported_v4_distribution() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../morphir-core/tests/fixtures/ir/v4/v4-library-distribution.json"
    ))
    .expect("the canonical v4 library distribution fixture should be valid JSON")
}

#[allow(dead_code)]
pub async fn assert_backend_session_conformance<S>(
    mut session: S,
    valid_ir: serde_json::Value,
    invalid_ir: serde_json::Value,
) -> S
where
    S: ExtensionSession,
{
    assert_eq!(session.state(), ExtensionSessionState::Starting);
    let error = session
        .invoke(methods::GENERATE, json!({}))
        .await
        .expect_err("operations should be rejected before initialization");
    assert!(error.to_string().contains("not ready"));

    let initialized = session
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "morphir-conformance".into(),
                version: "0.1.0".into(),
            },
        })
        .await
        .expect("the host and extension should negotiate MEP 0.1");
    assert_eq!(initialized.protocol_version, "0.1");
    assert!(
        initialized
            .extension
            .types
            .contains(&ExtensionType::Backend)
    );
    assert_eq!(session.state(), ExtensionSessionState::Ready);

    for lifecycle_method in [methods::INITIALIZE, methods::SHUTDOWN] {
        let error = session
            .invoke(lifecycle_method, json!({}))
            .await
            .expect_err("lifecycle methods should use their dedicated session operations");
        assert!(error.to_string().contains("lifecycle method"));
        assert_eq!(session.state(), ExtensionSessionState::Ready);
    }

    let generated = session
        .invoke(
            methods::GENERATE,
            serde_json::to_value(GenerateRequest {
                ir: valid_ir,
                target: "conformance".into(),
                options: Default::default(),
            })
            .expect("the valid generation request should serialize"),
        )
        .await
        .expect("an advertised backend should accept generation requests");
    let generated: GenerateResult =
        serde_json::from_value(generated).expect("generation should return a typed result");
    assert!(generated.success);
    assert!(!generated.artifacts.is_empty());
    assert!(generated.diagnostics.is_empty());

    let failed = session
        .invoke(
            methods::GENERATE,
            serde_json::to_value(GenerateRequest {
                ir: invalid_ir,
                target: "conformance".into(),
                options: Default::default(),
            })
            .expect("the invalid generation request should serialize"),
        )
        .await
        .expect("source failures should remain operation results");
    let failed: GenerateResult =
        serde_json::from_value(failed).expect("generation failure should return diagnostics");
    assert!(!failed.success);
    assert!(failed.artifacts.is_empty());
    assert!(!failed.diagnostics.is_empty());

    let error = session
        .invoke(
            methods::VALIDATE,
            serde_json::to_value(ValidateRequest {
                ir: json!({}),
                options: Default::default(),
            })
            .expect("the validation request should serialize"),
        )
        .await
        .expect_err("the host should reject capabilities the extension did not advertise");
    assert!(
        error
            .to_string()
            .contains(&error_codes::CAPABILITY_UNAVAILABLE.to_string())
    );

    session
        .shutdown()
        .await
        .expect("the initialized extension should shut down cleanly");
    assert_eq!(session.state(), ExtensionSessionState::Stopped);
    let error = session
        .invoke(methods::GENERATE, json!({}))
        .await
        .expect_err("operations should be rejected after shutdown");
    assert!(error.to_string().contains("not ready"));

    session
}

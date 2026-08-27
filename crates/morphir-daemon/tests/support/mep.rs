use morphir_daemon::extensions::{ExtensionSession, ExtensionSessionState, protocol::methods};
use morphir_extension_sdk::{
    ExtensionType, GenerateRequest, GenerateResult, ValidateRequest,
    protocol::{InitializeParams, PeerInfo, error_codes},
};
use serde_json::json;

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

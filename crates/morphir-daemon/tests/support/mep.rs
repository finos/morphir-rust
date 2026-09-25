//! The MEP lifecycle driver that `connected_daemon_extension` still uses.
//!
//! The other drivers moved to `morphir-host-native/tests/support/mep.rs` with
//! the real-extension tests.

use morphir_daemon::extensions::{InvokeOutcome, protocol::methods};
use morphir_extension_sdk::{
    ExtensionType, GenerateRequest, GenerateResult,
    protocol::{InitializeParams, PeerInfo},
};

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
                kind: Default::default(),
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

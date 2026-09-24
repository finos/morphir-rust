//! Installed claims select the expectation used during native host activation.

// The daemon also uses these fixtures; this target only needs the multi-document guests.
#[allow(dead_code)]
#[path = "support/activation.rs"]
mod runtime_mother;

use morphir_extension_sdk::protocol::{InitializeParams, PeerInfo};
use runtime_mother::{InstalledFrontend, RuntimeArtifact};

fn activation_params() -> InitializeParams {
    InitializeParams {
        protocol_versions: vec!["0.1".into()],
        host: PeerInfo {
            kind: Default::default(),
            name: "activation-test".into(),
            version: "1.0.0".into(),
        },
    }
}

async fn host_native_capabilities(
    fixture: RuntimeArtifact,
) -> Result<morphir_extension_sdk::ExtensionCapabilities, String> {
    use morphir_host::GuestConnection as _;
    let mut guest = morphir_host_native::activate(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap();
    guest
        .connection
        .open(activation_params())
        .await
        .map(|negotiated| negotiated.capabilities().clone())
        .map_err(|error| error.to_string())
}

#[tokio::test]
#[cfg(unix)]
async fn host_native_process_activation_multi_document_accepts_matching_claims() {
    let fixture =
        runtime_mother::process_with_multi_document(InstalledFrontend::ClaimsWithMultiDocument);
    let result = host_native_capabilities(fixture).await;
    let capabilities = result.expect("the installed frontend should accept multiDocument");
    assert!(capabilities.frontend.unwrap().multi_document);
}

#[tokio::test]
#[cfg(unix)]
async fn host_native_process_activation_multi_document_rejects_omitted_claim() {
    let fixture =
        runtime_mother::process_with_multi_document(InstalledFrontend::ClaimsWithoutMultiDocument);
    let result = host_native_capabilities(fixture).await;
    let error = result.expect_err("supplied claims default multiDocument to false and lock it");
    assert!(
        error.contains("capabilities disagreed with discovery"),
        "{error}"
    );
    assert!(error.contains("multiDocument"), "{error}");
}

#[tokio::test]
#[cfg(unix)]
async fn host_native_process_activation_multi_document_accepts_legacy() {
    let fixture = runtime_mother::process_with_multi_document(InstalledFrontend::Legacy);
    let result = host_native_capabilities(fixture).await;
    let capabilities = result.expect("the installed frontend should accept multiDocument");
    assert!(capabilities.frontend.unwrap().multi_document);
}

#[tokio::test]
async fn host_native_wasm_activation_multi_document_accepts_matching_claims() {
    let fixture =
        runtime_mother::wasm_with_multi_document(InstalledFrontend::ClaimsWithMultiDocument);
    let result = host_native_capabilities(fixture).await;
    let capabilities = result.expect("the installed frontend should accept multiDocument");
    assert!(capabilities.frontend.unwrap().multi_document);
}

#[tokio::test]
async fn host_native_wasm_activation_multi_document_rejects_omitted_claim() {
    let fixture =
        runtime_mother::wasm_with_multi_document(InstalledFrontend::ClaimsWithoutMultiDocument);
    let result = host_native_capabilities(fixture).await;
    let error = result.expect_err("supplied claims default multiDocument to false and lock it");
    assert!(
        error.contains("capabilities disagreed with discovery"),
        "{error}"
    );
    assert!(error.contains("multiDocument"), "{error}");
}

#[tokio::test]
async fn host_native_wasm_activation_multi_document_accepts_legacy() {
    let fixture = runtime_mother::wasm_with_multi_document(InstalledFrontend::Legacy);
    let result = host_native_capabilities(fixture).await;
    let capabilities = result.expect("the installed frontend should accept multiDocument");
    assert!(capabilities.frontend.unwrap().multi_document);
}

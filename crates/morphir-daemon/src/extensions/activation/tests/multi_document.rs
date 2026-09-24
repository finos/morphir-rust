//! Installed claims select the scope used by daemon activation.

use super::{RuntimeArtifact, activate_transport, activation_params, runtime_mother};
use runtime_mother::InstalledFrontend;

async fn daemon_capabilities(
    fixture: RuntimeArtifact,
) -> Result<morphir_extension_sdk::ExtensionCapabilities, String> {
    let negotiation = activate_transport(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap()
        .initialize(activation_params())
        .await;
    match negotiation {
        Ok(ready) => Ok(ready.negotiated().capabilities().clone()),
        Err(failure) => Err(failure.error().to_string()),
    }
}

#[tokio::test]
#[cfg(unix)]
async fn daemon_process_activation_multi_document_accepts_matching_claims() {
    let fixture =
        runtime_mother::process_with_multi_document(InstalledFrontend::ClaimsWithMultiDocument);
    let result = daemon_capabilities(fixture).await;
    let capabilities = result.expect("the installed frontend should accept multiDocument");
    assert!(capabilities.frontend.unwrap().multi_document);
}

#[tokio::test]
#[cfg(unix)]
async fn daemon_process_activation_multi_document_rejects_omitted_claim() {
    let fixture =
        runtime_mother::process_with_multi_document(InstalledFrontend::ClaimsWithoutMultiDocument);
    let result = daemon_capabilities(fixture).await;
    let error = result.expect_err("supplied claims default multiDocument to false and lock it");
    assert!(
        error.contains("capabilities disagreed with discovery"),
        "{error}"
    );
    assert!(error.contains("multiDocument"), "{error}");
}

#[tokio::test]
#[cfg(unix)]
async fn daemon_process_activation_multi_document_accepts_legacy() {
    let fixture = runtime_mother::process_with_multi_document(InstalledFrontend::Legacy);
    let result = daemon_capabilities(fixture).await;
    let capabilities = result.expect("the installed frontend should accept multiDocument");
    assert!(capabilities.frontend.unwrap().multi_document);
}

#[tokio::test]
async fn daemon_wasm_activation_multi_document_accepts_matching_claims() {
    let fixture =
        runtime_mother::wasm_with_multi_document(InstalledFrontend::ClaimsWithMultiDocument);
    let result = daemon_capabilities(fixture).await;
    let capabilities = result.expect("the installed frontend should accept multiDocument");
    assert!(capabilities.frontend.unwrap().multi_document);
}

#[tokio::test]
async fn daemon_wasm_activation_multi_document_rejects_omitted_claim() {
    let fixture =
        runtime_mother::wasm_with_multi_document(InstalledFrontend::ClaimsWithoutMultiDocument);
    let result = daemon_capabilities(fixture).await;
    let error = result.expect_err("supplied claims default multiDocument to false and lock it");
    assert!(
        error.contains("capabilities disagreed with discovery"),
        "{error}"
    );
    assert!(error.contains("multiDocument"), "{error}");
}

#[tokio::test]
async fn daemon_wasm_activation_multi_document_accepts_legacy() {
    let fixture = runtime_mother::wasm_with_multi_document(InstalledFrontend::Legacy);
    let result = daemon_capabilities(fixture).await;
    let capabilities = result.expect("the installed frontend should accept multiDocument");
    assert!(capabilities.frontend.unwrap().multi_document);
}

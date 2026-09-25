//! Installed claims select the expectation used during native host activation.

// The daemon also uses these fixtures; this target does not need all of them.
#[allow(dead_code)]
#[path = "support/activation.rs"]
mod runtime_mother;

use morphir_extension_sdk::protocol::{InitializeParams, PeerInfo};
use morphir_extension_sdk::{
    BackendCapability, ExtensionCapabilities, ExtensionType, FrontendCapability, LanguageCapability,
};
use morphir_host::{GuestConnection as _, PersistedExtensionCapabilities};
use runtime_mother::{InstalledFrontend, RuntimeArtifact};
use std::fs;
#[cfg(unix)]
use std::path::Path;

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

// Activation of installed process and WebAssembly guests, ported from the
// daemon's activation tests.

fn expected_backend_capability() -> BackendCapability {
    BackendCapability {
        targets: vec!["avro".into(), "json-schema".into()],
        ir_versions: vec!["3".into(), "4".into()],
        generate: true,
    }
}

fn expected_capabilities() -> ExtensionCapabilities {
    ExtensionCapabilities {
        frontend: Some(FrontendCapability {
            languages: vec![LanguageCapability {
                id: "gleam".into(),
                file_extensions: vec![".gleam".into()],
            }],
            ir_versions: vec!["4".into()],
            compile: true,
            incremental: false,
            fragments: false,
            multi_document: false,
        }),
        backend: Some(expected_backend_capability()),
        ..ExtensionCapabilities::default()
    }
}

fn expected_persisted_capabilities() -> PersistedExtensionCapabilities {
    let capabilities = expected_capabilities();
    PersistedExtensionCapabilities::new(capabilities.frontend, capabilities.backend)
}

#[cfg(unix)]
async fn wait_for_launch(path: &Path, expected_lines: usize) -> String {
    // Up to ten seconds: a shell child can start slowly while other tests launch processes in
    // parallel. A launch that is quick returns at once.
    for _ in 0..1000 {
        if let Ok(output) = fs::read_to_string(path)
            && output.lines().count() >= expected_lines
        {
            return output;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("spawned process did not record its launch")
}

#[tokio::test]
#[cfg(unix)]
async fn process_activation_preserves_discovery_and_persisted_capabilities() {
    let (fixture, capture, args) = runtime_mother::process();
    let expected_working_directory = fs::canonicalize(&fixture.working_directory).unwrap();
    let staging_directory = fixture.staging_directory.clone();
    let guest = morphir_host_native::activate(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap();

    assert_eq!(fs::read_dir(staging_directory).unwrap().count(), 1);

    let expected = &guest.expectation;
    assert_eq!(expected.id(), "morphir-process");
    let discovered = expected.extension_info().unwrap();
    assert_eq!(discovered.name, "Morphir Process");
    assert_eq!(discovered.version, "1.2.3");
    assert_eq!(
        discovered.types,
        [ExtensionType::Frontend, ExtensionType::Backend]
    );
    assert_eq!(
        expected.backend_capability(),
        Some(&expected_backend_capability())
    );
    assert_eq!(
        expected.persisted_capabilities(),
        Some(&expected_persisted_capabilities())
    );
    assert!(expected.capabilities().is_none());

    let observed = wait_for_launch(&capture, args.len() + 2).await;
    let mut lines = observed.lines();
    assert_eq!(
        lines.next(),
        Some(expected_working_directory.to_str().unwrap())
    );
    assert_eq!(lines.next(), Some(args.len().to_string().as_str()));
    assert_eq!(
        lines.collect::<Vec<_>>(),
        args.iter()
            .map(|argument| format!("<{argument}>"))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
#[cfg(unix)]
async fn process_activation_uses_the_exact_bytes_verified_before_store_replacement() {
    let (fixture, capture, args) = runtime_mother::process();
    fs::write(
        &fixture.installed_path,
        b"#!/bin/sh\nprintf 'replacement\\n' > \"$1\"\nwhile IFS= read -r line; do :; done\n",
    )
    .unwrap();

    let _guest = morphir_host_native::activate(fixture.artifact, &fixture.working_directory)
        .await
        .expect("activation should stage and execute the already verified process bytes");

    let observed = wait_for_launch(&capture, args.len() + 2).await;
    assert_ne!(observed.trim(), "replacement");
    assert_eq!(
        observed.lines().next(),
        fs::canonicalize(&fixture.working_directory)
            .unwrap()
            .to_str()
    );
}

#[tokio::test]
#[cfg(unix)]
async fn backend_only_process_activation_uses_persisted_backend_metadata() {
    let fixture = runtime_mother::backend_process();
    let guest = morphir_host_native::activate(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap();

    let expected = &guest.expectation;
    assert_eq!(expected.id(), "morphir-backend");
    assert_eq!(
        expected.extension_info().unwrap().types,
        [ExtensionType::Backend]
    );
    assert_eq!(
        expected.backend_capability(),
        Some(&expected_backend_capability())
    );
    assert_eq!(
        expected.persisted_capabilities(),
        Some(&PersistedExtensionCapabilities::new(
            None,
            Some(expected_backend_capability())
        ))
    );
    assert!(expected.capabilities().is_none());
}

#[tokio::test]
#[cfg(unix)]
async fn process_activation_allows_unpersisted_workspace_capabilities() {
    let fixture = runtime_mother::process_with_frontend_workspace(true);
    let result = host_native_capabilities(fixture).await;

    match result {
        Ok(capabilities) => assert!(capabilities.workspace.is_some()),
        Err(error) => {
            panic!("unpersisted workspace capabilities should remain negotiable: {error}")
        }
    }
}

#[tokio::test]
#[cfg(unix)]
async fn process_activation_rejects_frontend_capability_drift() {
    let fixture = runtime_mother::process_with_frontend_workspace(false);
    let error = host_native_capabilities(fixture)
        .await
        .expect_err("frontend capability drift should fail initialization");

    assert!(
        error.contains("capabilities disagreed with discovery"),
        "{error}"
    );
}

#[tokio::test]
async fn wasm_activation_uses_locked_identity_for_the_shared_loaded_transport() {
    let fixture = runtime_mother::wasm();
    let mut guest = morphir_host_native::activate(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap();

    let expected = &guest.expectation;
    assert_eq!(expected.id(), "morphir-avro");
    let discovered = expected.extension_info().unwrap();
    assert_eq!(discovered.name, "Morphir Avro");
    assert_eq!(discovered.version, "1.2.3");
    assert_eq!(
        discovered.types,
        [ExtensionType::Frontend, ExtensionType::Backend]
    );
    assert_eq!(
        expected.backend_capability(),
        Some(&expected_backend_capability())
    );
    assert_eq!(
        expected.persisted_capabilities(),
        Some(&expected_persisted_capabilities())
    );
    assert!(expected.capabilities().is_none());

    let error = guest
        .connection
        .open(activation_params())
        .await
        .expect_err("guest identity drift should fail initialization")
        .to_string();
    assert!(
        error.contains("identity changed during initialization"),
        "{error}"
    );
    assert!(error.contains("morphir-avro"), "{error}");
    assert!(error.contains("guest-self-report"), "{error}");
}

#[tokio::test]
async fn wasm_activation_allows_unpersisted_workspace_capabilities() {
    let fixture = runtime_mother::wasm_with_frontend_workspace(true);
    let result = host_native_capabilities(fixture).await;

    match result {
        Ok(capabilities) => assert!(capabilities.workspace.is_some()),
        Err(error) => {
            panic!("unpersisted workspace capabilities should remain negotiable: {error}")
        }
    }
}

#[tokio::test]
async fn wasm_activation_uses_the_exact_bytes_verified_before_store_replacement() {
    let fixture = runtime_mother::wasm();
    fs::write(&fixture.installed_path, b"replacement wasm bytes").unwrap();

    let mut guest = morphir_host_native::activate(fixture.artifact, &fixture.working_directory)
        .await
        .expect("activation should consume the already verified wasm bytes");

    assert_eq!(guest.id, "morphir-avro");
    // The verified module still runs: it reports its own identity, which the
    // installed record locks against.
    let error = guest
        .connection
        .open(activation_params())
        .await
        .expect_err("the verified guest reports its own identity")
        .to_string();
    assert!(error.contains("guest-self-report"), "{error}");
}

#[tokio::test]
async fn host_native_wasm_activation_negotiates_persisted_capabilities() {
    let fixture = runtime_mother::wasm_with_capabilities(true);
    let mut guest = morphir_host_native::activate(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap();

    assert_eq!(guest.id, "morphir-capabilities");
    let negotiated = guest
        .connection
        .open(activation_params())
        .await
        .expect("the guest should reproduce both installed capabilities");
    assert_eq!(negotiated.capabilities(), &expected_capabilities());
}

#[tokio::test]
async fn host_native_wasm_activation_uses_the_locked_identity() {
    let fixture = runtime_mother::wasm();
    let mut guest = morphir_host_native::activate(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap();

    assert_eq!(guest.id, "morphir-avro");
    let error = guest
        .connection
        .open(activation_params())
        .await
        .expect_err("guest identity drift should fail initialization")
        .to_string();
    assert!(
        error.contains("identity changed during initialization"),
        "{error}"
    );
    assert!(error.contains("morphir-avro"), "{error}");
    assert!(error.contains("guest-self-report"), "{error}");
}

#[tokio::test]
async fn host_native_wasm_activation_rejects_frontend_capability_drift() {
    let fixture = runtime_mother::wasm_with_frontend_workspace(false);
    let mut guest = morphir_host_native::activate(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap();

    let error = guest
        .connection
        .open(activation_params())
        .await
        .expect_err("frontend capability drift should fail initialization")
        .to_string();
    assert!(
        error.contains("capabilities disagreed with discovery"),
        "{error}"
    );
}

#[tokio::test]
#[cfg(unix)]
async fn host_native_process_activation_passes_arguments_and_working_directory() {
    let (fixture, capture, args) = runtime_mother::process();
    let expected_working_directory = fs::canonicalize(&fixture.working_directory).unwrap();
    let guest = morphir_host_native::activate(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap();

    assert_eq!(guest.id, "morphir-process");
    let observed = wait_for_launch(&capture, args.len() + 2).await;
    let mut lines = observed.lines();
    assert_eq!(
        lines.next(),
        Some(expected_working_directory.to_str().unwrap())
    );
    assert_eq!(lines.next(), Some(args.len().to_string().as_str()));
    assert_eq!(
        lines.collect::<Vec<_>>(),
        args.iter()
            .map(|argument| format!("<{argument}>"))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
#[cfg(unix)]
async fn host_native_process_activation_negotiates_persisted_capabilities() {
    let fixture = runtime_mother::process_with_capabilities(true);
    let mut guest = morphir_host_native::activate(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap();

    let negotiated = guest
        .connection
        .open(activation_params())
        .await
        .expect("the process should reproduce both installed capabilities");
    assert_eq!(negotiated.capabilities(), &expected_capabilities());
}

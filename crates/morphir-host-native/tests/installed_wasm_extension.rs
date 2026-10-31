//! Lifecycle conformance for a release-built installed WebAssembly extension.
//!
//! Build the fixture before running this ignored test:
//!
//! `cargo build --release -p morphir-rust-binding --target wasm32-unknown-unknown`
//! `cargo test -p morphir-host-native --test installed_wasm_extension -- --ignored`

mod support;

use morphir_distribution::activate_installed;
use morphir_extension_sdk::{
    DiagnosticSeverity, ExtensionType, GenerateRequest, GenerateResult, protocol::methods,
};
use morphir_host::{CallError, Session};
use serde_json::json;

use support::installed_wasm::{InstalledWasmMother, crate_version, wasm_guest_path};
use support::mep::{completed, host_config};

#[test]
fn cargo_metadata_resolves_an_absolute_target_directory() {
    assert!(support::installed_wasm::cargo_target_directory().is_absolute());
}

#[tokio::test]
#[ignore = "requires a release wasm guest"]
async fn installed_wasm_runs_the_common_mep_lifecycle() {
    // Read from the manifest rather than repeated as a literal: the index
    // record and the guest's own initialization metadata have to agree, so a
    // crate version bump would otherwise fail negotiation here.
    let version = crate_version("morphir-rust-binding");
    let fixture = InstalledWasmMother::from_path(
        wasm_guest_path("morphir_rust_binding.wasm"),
        "morphir-rust",
        "Morphir Rust",
        &version,
        &["rust"],
        &["3", "4"],
        Some(json!({
            "languages": [{ "id": "rust", "fileExtensions": [".rs"] }],
            "irVersions": ["3", "4"],
            "compile": true
        })),
    );
    let installed = fixture
        .install()
        .expect("the release guest should install securely");
    let guest = morphir_host_native::activate(
        activate_installed(&fixture.home, installed.extension_id())
            .expect("the installed guest should activate offline"),
        fixture.workspace.path(),
    )
    .await
    .expect("the installed guest should load through the host");

    let mut session = Session::open(
        guest.connection,
        &host_config("morphir-installed-wasm-conformance", "0.1.0"),
    )
    .await
    .unwrap_or_else(|error| panic!("MEP negotiation failed: {error}"));

    assert_eq!(session.negotiated().protocol_version(), "0.1");
    let info = session.negotiated().extension();
    assert_eq!(info.id, "morphir-rust");
    assert_eq!(info.name, "Morphir Rust");
    assert_eq!(info.version, version);
    assert_eq!(
        info.types,
        [ExtensionType::Frontend, ExtensionType::Backend]
    );
    let backend = session
        .negotiated()
        .capabilities()
        .backend
        .as_ref()
        .expect("the locked backend capability should negotiate");
    assert_eq!(backend.targets, ["rust"]);
    assert_eq!(backend.ir_versions, ["3", "4"]);
    assert!(backend.generate);

    let valid_request = GenerateRequest {
        ir: supported_v4_distribution(),
        target: "rust".into(),
        options: Default::default(),
    };
    let generated = completed(
        "valid generation",
        session.generate(valid_request.clone()).await,
    );
    assert_generated_rust(&generated);

    let invalid_request = GenerateRequest {
        ir: json!({ "formatVersion": 5, "distribution": null }),
        target: "rust".into(),
        options: Default::default(),
    };
    let generated = completed(
        "domain-invalid generation",
        session.generate(invalid_request).await,
    );
    assert!(!generated.success);
    assert!(generated.artifacts.is_empty());
    assert_eq!(generated.diagnostics.len(), 1);
    assert_eq!(generated.diagnostics[0].code.as_deref(), Some("RS_VERSION"));
    assert_eq!(generated.diagnostics[0].severity, DiagnosticSeverity::Error);

    match session
        .call::<_, GenerateResult>(methods::GENERATE, json!({ "options": "not an option map" }))
        .await
    {
        Err(CallError::Rejected(error)) => assert!(
            error.to_string().contains("-32602"),
            "expected JSON-RPC invalid params, got {error}"
        ),
        Ok(_) => panic!("malformed generate params should be rejected"),
        Err(error) => panic!("malformed params broke the MEP session: {error:?}"),
    }

    let generated = completed(
        "valid generation after rejection",
        session.generate(valid_request).await,
    );
    assert_generated_rust(&generated);

    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("MEP shutdown failed: {error}"));
}

fn supported_v4_distribution() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../morphir-projection/tests/fixtures/canonical-customer-v4.json"
    ))
    .expect("the canonical customer v4 fixture should be valid JSON")
}

fn assert_generated_rust(generated: &GenerateResult) {
    assert!(generated.success, "{:?}", generated.diagnostics);
    assert!(!generated.artifacts.is_empty());
    assert!(generated.artifacts.iter().all(|artifact| {
        artifact.path.ends_with(".rs") && !artifact.binary && !artifact.content.is_empty()
    }));
}

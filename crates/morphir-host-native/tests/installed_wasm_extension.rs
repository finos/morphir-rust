//! Lifecycle conformance for a release-built installed WebAssembly extension.
//!
//! Build the fixture before running this ignored test:
//!
//! `cargo build --release -p morphir-avro-extension --target wasm32-unknown-unknown`
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
    let version = crate_version("morphir-avro-extension");
    let fixture = InstalledWasmMother::from_path(
        wasm_guest_path("morphir_avro_extension.wasm"),
        "morphir-avro",
        "Morphir Avro",
        &version,
        &["avro"],
        &["3", "4"],
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
    assert_eq!(info.id, "morphir-avro");
    assert_eq!(info.name, "Morphir Avro");
    assert_eq!(info.version, version);
    assert_eq!(info.types, [ExtensionType::Backend]);
    let backend = session
        .negotiated()
        .capabilities()
        .backend
        .as_ref()
        .expect("the locked backend capability should negotiate");
    assert_eq!(backend.targets, ["avro"]);
    assert_eq!(backend.ir_versions, ["3", "4"]);
    assert!(backend.generate);

    let valid_request = GenerateRequest {
        ir: supported_v4_distribution(),
        target: "avro".into(),
        options: [
            ("representation".into(), json!("idl")),
            ("projection".into(), json!("protocol-public")),
            (
                "type_mappings".into(),
                // The keys are FQNames, and an FQName is matched by the name it
                // spells, not by the characters. `morphir/SDK` is how the
                // canonical fixture -- and the Avro backend's own SDK table --
                // spells the SDK package; `morphir/sdk` would name a different
                // package and quietly override nothing.
                json!({
                    "morphir/SDK:string#string": { "type": "bytes" },
                    "morphir/SDK:basics#int": { "type": "double" }
                }),
            ),
        ]
        .into_iter()
        .collect(),
    };
    let generated = completed(
        "valid generation",
        session.generate(valid_request.clone()).await,
    );
    assert!(generated.success, "{:?}", generated.diagnostics);
    assert!(!generated.artifacts.is_empty());
    assert!(generated.artifacts.iter().all(|artifact| {
        artifact.path.ends_with(".avdl")
            && !artifact.binary
            && artifact.content.contains("protocol ")
    }));
    assert_configured_type_mappings(&generated);

    let invalid_request = GenerateRequest {
        ir: json!({ "formatVersion": 5, "distribution": null }),
        target: "avro".into(),
        options: Default::default(),
    };
    let generated = completed(
        "domain-invalid generation",
        session.generate(invalid_request).await,
    );
    assert!(!generated.success);
    assert!(generated.artifacts.is_empty());
    assert_eq!(generated.diagnostics.len(), 1);
    assert_eq!(
        generated.diagnostics[0].code.as_deref(),
        Some("unsupported_format_version_major")
    );
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
    assert!(generated.success, "{:?}", generated.diagnostics);
    assert!(generated.artifacts.iter().any(|artifact| {
        artifact.path.ends_with(".avdl")
            && !artifact.binary
            && artifact.content.contains("protocol ")
    }));
    assert_configured_type_mappings(&generated);

    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("MEP shutdown failed: {error}"));
}

fn supported_v4_distribution() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../morphir-core/tests/fixtures/ir/v4/v4-library-distribution.json"
    ))
    .expect("the canonical v4 library distribution fixture should be valid JSON")
}

fn assert_configured_type_mappings(generated: &GenerateResult) {
    let idl = generated
        .artifacts
        .iter()
        .map(|artifact| artifact.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        idl.contains("bytes getUserName("),
        "configured SDK String mapping was not rendered: {idl}"
    );
    assert!(
        idl.contains("double nativeAdd(") && idl.contains("double a") && idl.contains("double b"),
        "configured SDK Int mapping was not rendered: {idl}"
    );
}

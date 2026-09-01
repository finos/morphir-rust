//! Lifecycle conformance for a release-built installed WebAssembly extension.
//!
//! Build the fixture before running this ignored test:
//!
//! `cargo build --release -p morphir-avro-extension --target wasm32-unknown-unknown`
//! `cargo test -p morphir-daemon --test installed_wasm_extension -- --ignored`

mod support;

use morphir_daemon::extensions::{InvokeOutcome, activate_transport, protocol::methods};
use morphir_distribution::activate_installed;
use serde_json::json;

use morphir_extension_sdk::{
    DiagnosticSeverity, ExtensionType, GenerateRequest, GenerateResult,
    protocol::{InitializeParams, PeerInfo},
};

use support::installed_wasm::{InstalledWasmMother, crate_version, wasm_guest_path};

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
    let loaded = activate_transport(
        activate_installed(&fixture.home, installed.extension_id())
            .expect("the installed guest should activate offline"),
        fixture.workspace.path(),
    )
    .await
    .expect("the installed guest should load through the daemon");

    let ready = loaded
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "morphir-installed-wasm-conformance".into(),
                version: "0.1.0".into(),
            },
        })
        .await
        .unwrap_or_else(|failure| panic!("MEP negotiation failed: {}", failure.error()));

    assert_eq!(ready.negotiated().protocol_version(), "0.1");
    let info = ready.negotiated().extension();
    assert_eq!(info.id, "morphir-avro");
    assert_eq!(info.name, "Morphir Avro");
    assert_eq!(info.version, version);
    assert_eq!(info.types, [ExtensionType::Backend]);
    let backend = ready
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
                json!({
                    "morphir/sdk:string#string": { "type": "bytes" },
                    "morphir/sdk:basics#int": { "type": "double" }
                }),
            ),
        ]
        .into_iter()
        .collect(),
    };
    let ready = match ready
        .invoke::<GenerateResult>(methods::GENERATE, valid_request.clone())
        .await
    {
        InvokeOutcome::Success(ready, generated) => {
            assert!(generated.success, "{:?}", generated.diagnostics);
            assert!(!generated.artifacts.is_empty());
            assert!(generated.artifacts.iter().all(|artifact| {
                artifact.path.ends_with(".avdl")
                    && !artifact.binary
                    && artifact.content.contains("protocol ")
            }));
            assert_configured_type_mappings(&generated);
            ready
        }
        InvokeOutcome::Rejected(_, error) => panic!("valid generation was rejected: {error}"),
        InvokeOutcome::Failed(failure) => {
            panic!("valid generation failed MEP: {}", failure.error())
        }
    };

    let invalid_request = GenerateRequest {
        ir: json!({ "formatVersion": 5, "distribution": null }),
        target: "avro".into(),
        options: Default::default(),
    };
    let ready = match ready
        .invoke::<GenerateResult>(methods::GENERATE, invalid_request)
        .await
    {
        InvokeOutcome::Success(ready, generated) => {
            assert!(!generated.success);
            assert!(generated.artifacts.is_empty());
            assert_eq!(generated.diagnostics.len(), 1);
            assert_eq!(
                generated.diagnostics[0].code.as_deref(),
                Some("unsupported_format_version_major")
            );
            assert_eq!(generated.diagnostics[0].severity, DiagnosticSeverity::Error);
            ready
        }
        InvokeOutcome::Rejected(_, error) => {
            panic!("domain-invalid generation was rejected at MEP: {error}")
        }
        InvokeOutcome::Failed(failure) => {
            panic!("domain-invalid generation failed MEP: {}", failure.error())
        }
    };

    let ready = match ready
        .invoke::<GenerateResult>(methods::GENERATE, json!({ "options": "not an option map" }))
        .await
    {
        InvokeOutcome::Rejected(ready, error) => {
            assert!(
                error.to_string().contains("-32602"),
                "expected JSON-RPC invalid params, got {error}"
            );
            ready
        }
        InvokeOutcome::Success(_, _) => panic!("malformed generate params should be rejected"),
        InvokeOutcome::Failed(failure) => {
            panic!(
                "malformed params broke the MEP session: {}",
                failure.error()
            )
        }
    };

    let ready = match ready
        .invoke::<GenerateResult>(methods::GENERATE, valid_request)
        .await
    {
        InvokeOutcome::Success(ready, generated) => {
            assert!(generated.success, "{:?}", generated.diagnostics);
            assert!(generated.artifacts.iter().any(|artifact| {
                artifact.path.ends_with(".avdl")
                    && !artifact.binary
                    && artifact.content.contains("protocol ")
            }));
            assert_configured_type_mappings(&generated);
            ready
        }
        InvokeOutcome::Rejected(_, error) => {
            panic!("valid generation after rejection was rejected: {error}")
        }
        InvokeOutcome::Failed(failure) => {
            panic!(
                "valid generation after rejection failed MEP: {}",
                failure.error()
            )
        }
    };

    ready
        .shutdown()
        .await
        .unwrap_or_else(|failure| panic!("MEP shutdown failed: {}", failure.error()));
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

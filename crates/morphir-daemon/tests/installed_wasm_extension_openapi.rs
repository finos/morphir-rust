//! Lifecycle conformance for a release-built installed OpenAPI WebAssembly
//! extension: it must install, negotiate the MEP handshake, and run a real
//! generation through the compiled component under the host runtime — not
//! just in-process as native Rust.
//!
//! Build the fixture before running this ignored test:
//!
//! `cargo build --release -p morphir-openapi-extension --target wasm32-unknown-unknown`
//! `cargo test -p morphir-daemon --test installed_wasm_extension_openapi -- --ignored`

mod support;

use std::collections::HashMap;

use morphir_daemon::extensions::{InvokeOutcome, activate_transport, protocol::methods};
use morphir_distribution::activate_installed;
use morphir_extension_sdk::{
    ExtensionType, GenerateRequest, GenerateResult,
    protocol::{InitializeParams, PeerInfo},
};
use morphir_projection::testing::classic_schema_library;

use support::installed_wasm::{InstalledWasmMother, crate_version, wasm_guest_path};

#[tokio::test]
#[ignore = "requires a release wasm guest"]
async fn installed_openapi_wasm_runs_one_generation_through_the_host() {
    // Read from the manifest rather than repeated as a literal: the index
    // record and the guest's own initialization metadata have to agree, so a
    // crate version bump would otherwise fail negotiation here.
    let version = crate_version("morphir-openapi-extension");
    let fixture = InstalledWasmMother::from_path(
        wasm_guest_path("morphir_openapi_extension.wasm"),
        "morphir-openapi",
        "Morphir OpenAPI",
        &version,
        &["openapi", "json-schema"],
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
    assert_eq!(info.id, "morphir-openapi");
    assert_eq!(info.name, "Morphir OpenAPI");
    assert_eq!(info.version, version);
    assert_eq!(info.types, [ExtensionType::Backend]);
    let backend = ready
        .negotiated()
        .capabilities()
        .backend
        .as_ref()
        .expect("the locked backend capability should negotiate");
    assert_eq!(backend.targets, ["openapi", "json-schema"]);
    assert_eq!(backend.ir_versions, ["3", "4"]);
    assert!(backend.generate);

    let request = GenerateRequest {
        ir: classic_schema_library(),
        target: "json-schema".into(),
        options: HashMap::new(),
    };
    let ready =
        match ready
            .invoke::<GenerateResult>(methods::GENERATE, request)
            .await
        {
            InvokeOutcome::Success(ready, generated) => {
                assert!(generated.success, "{:?}", generated.diagnostics);
                assert!(!generated.artifacts.is_empty());
                assert!(generated.artifacts.iter().all(|artifact| {
                    artifact.path.ends_with(".schema.json") && !artifact.binary
                }));
                ready
            }
            InvokeOutcome::Rejected(_, error) => panic!("valid generation was rejected: {error}"),
            InvokeOutcome::Failed(failure) => {
                panic!("valid generation failed MEP: {}", failure.error())
            }
        };

    ready
        .shutdown()
        .await
        .unwrap_or_else(|failure| panic!("MEP shutdown failed: {}", failure.error()));
}

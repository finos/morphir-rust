//! Lifecycle conformance for a release-built installed OpenAPI WebAssembly
//! extension: it must install, negotiate the MEP handshake, and run a real
//! generation through the compiled component under the host runtime — not
//! just in-process as native Rust.
//!
//! Build the fixture before running this ignored test:
//!
//! `cargo build --release -p morphir-openapi-extension --target wasm32-unknown-unknown`
//! `cargo test -p morphir-host-native --test installed_wasm_extension_openapi -- --ignored`

mod support;

use std::collections::HashMap;

use morphir_distribution::activate_installed;
use morphir_extension_sdk::{ExtensionType, GenerateRequest};
use morphir_host::Session;
use morphir_projection::testing::classic_schema_library;

use support::installed_wasm::{InstalledWasmMother, crate_version, wasm_guest_path};
use support::mep::{completed, host_config};

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
    assert_eq!(info.id, "morphir-openapi");
    assert_eq!(info.name, "Morphir OpenAPI");
    assert_eq!(info.version, version);
    assert_eq!(info.types, [ExtensionType::Backend]);
    let backend = session
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
    let generated = completed("valid generation", session.generate(request).await);
    assert!(generated.success, "{:?}", generated.diagnostics);
    assert!(!generated.artifacts.is_empty());
    assert!(
        generated
            .artifacts
            .iter()
            .all(|artifact| { artifact.path.ends_with(".schema.json") && !artifact.binary })
    );

    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("MEP shutdown failed: {error}"));
}

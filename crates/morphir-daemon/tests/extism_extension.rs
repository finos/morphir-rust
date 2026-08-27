//! Conformance tests for a real Extism extension artifact.
//!
//! Build the fixture before running these ignored tests:
//!
//! `cargo build --release -p morphir-wasm-binding --target wasm32-unknown-unknown`
//! `cargo test -p morphir-daemon --test extism_extension -- --ignored`

mod support;

use base64::{Engine, engine::general_purpose::STANDARD};
use morphir_daemon::ExtensionContainer;
use morphir_daemon::extensions::{
    container::ExtensionType, host_functions::MorphirHostFunctions, protocol::methods,
    session::ExtismSession,
};
use morphir_extension_sdk::{
    ExtensionCapabilities, GenerateRequest, GenerateResult, protocol::error_codes,
};
use serde_json::json;
use std::path::PathBuf;

struct ExtismConformanceDriver {
    container: ExtensionContainer,
}

impl ExtismConformanceDriver {
    fn load_wasm_backend() -> Self {
        let container = ExtensionContainer::new(
            "morphir-wasm-binding",
            &wasm_binding_path(),
            MorphirHostFunctions::default(),
        )
        .expect("the real Wasm backend should load through Extism");

        Self { container }
    }

    async fn generate(&self, ir: serde_json::Value) -> GenerateResult {
        self.container
            .call(
                methods::GENERATE,
                GenerateRequest {
                    ir,
                    options: Default::default(),
                },
            )
            .await
            .expect("the backend request should return a generation result")
    }
}

fn wasm_binding_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/wasm32-unknown-unknown/release/morphir_wasm_binding.wasm")
}

fn a_distribution_with_one_value() -> serde_json::Value {
    json!({
        "name": "conformance",
        "modules": [{
            "name": "main",
            "values": [{
                "name": "answer",
                "body": {
                    "kind": "literal",
                    "value": { "type": "int", "value": 42 }
                }
            }]
        }]
    })
}

#[tokio::test]
#[ignore = "requires the independently built morphir-wasm-binding artifact"]
async fn loads_the_real_extension_and_discovers_its_capabilities() {
    let driver = ExtismConformanceDriver::load_wasm_backend();

    assert_eq!(driver.container.info().id, "morphir-wasm-binding");
    assert!(driver.container.supports(ExtensionType::Backend));

    let capabilities: ExtensionCapabilities = driver
        .container
        .call(methods::CAPABILITIES, json!({}))
        .await
        .expect("the extension should report its capabilities through JSON-RPC");
    assert!(!capabilities.streaming);
}

#[tokio::test]
#[ignore = "requires the independently built morphir-wasm-binding artifact"]
async fn invokes_the_real_backend_and_returns_valid_wasm() {
    let driver = ExtismConformanceDriver::load_wasm_backend();

    let result = driver.generate(a_distribution_with_one_value()).await;

    assert!(result.success);
    assert!(result.diagnostics.is_empty());
    assert_eq!(result.artifacts.len(), 1);
    assert_eq!(result.artifacts[0].path, "conformance.wasm");
    assert!(result.artifacts[0].binary);
    let wasm = STANDARD
        .decode(&result.artifacts[0].content)
        .expect("the binary artifact should contain base64");
    assert!(wasm.starts_with(b"\0asm"));
}

#[tokio::test]
#[ignore = "requires the independently built morphir-wasm-binding artifact"]
async fn returns_generation_failures_as_structured_diagnostics() {
    let driver = ExtismConformanceDriver::load_wasm_backend();

    let result = driver.generate(json!("not Morphir IR")).await;

    assert!(!result.success);
    assert!(result.artifacts.is_empty());
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].code.as_deref(), Some("W001"));
}

#[tokio::test]
#[ignore = "requires the independently built morphir-wasm-binding artifact"]
async fn rejects_unknown_methods_at_the_guest_boundary() {
    let driver = ExtismConformanceDriver::load_wasm_backend();

    let error = driver
        .container
        .call::<_, serde_json::Value>("morphir.unknown", json!({}))
        .await
        .expect_err("the guest should reject an unknown method");

    assert!(
        error
            .to_string()
            .contains(&error_codes::METHOD_NOT_FOUND.to_string())
    );
}

#[tokio::test]
#[ignore = "requires the independently built morphir-wasm-binding artifact"]
async fn completes_the_mep_lifecycle_in_order() {
    let driver = ExtismConformanceDriver::load_wasm_backend();
    support::mep::assert_backend_session_conformance(
        ExtismSession::new(driver.container),
        a_distribution_with_one_value(),
        json!("not Morphir IR"),
    )
    .await;
}

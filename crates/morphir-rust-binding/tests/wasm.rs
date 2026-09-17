//! Opt-in tests of the actual Extism guest exports.
//!
//! Run from the workspace root:
//! ```text
//! cargo +1.98.0 build -p morphir-rust-binding --target wasm32-unknown-unknown
//! cargo +1.98.0 test -p morphir-rust-binding --features wasm-host-tests --test wasm -- --ignored
//! ```
//! Set MORPHIR_RUST_GUEST to test a release or externally built guest instead.

#![cfg(not(target_arch = "wasm32"))]

use extism::Plugin;
use morphir_extension_sdk::{
    prelude::*,
    protocol::{
        ExtensionRequest, ExtensionResponse, InitializeParams, InitializeResult, JSONRPC_VERSION,
        MEP_VERSION, PeerInfo, error_codes, methods,
    },
};
use morphir_rust_binding::RustExtension;
use serde_json::{Value, json};
use std::path::PathBuf;

fn guest() -> Plugin {
    let path = std::env::var_os("MORPHIR_RUST_GUEST")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/wasm32-unknown-unknown/debug/morphir_rust_binding.wasm")
        });
    let wasm = std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "cannot read guest {}: {error}; build it with cargo +1.98.0 build \
             -p morphir-rust-binding --target wasm32-unknown-unknown, \
             or set MORPHIR_RUST_GUEST",
            path.display()
        )
    });
    // The binding requires neither WASI nor filesystem/network host functions.
    Plugin::new(wasm, [], false).expect("the Rust guest must load without host imports")
}

fn call(plugin: &mut Plugin, request: ExtensionRequest) -> ExtensionResponse {
    let input = serde_json::to_vec(&request).unwrap();
    let output = plugin
        .call::<&[u8], &[u8]>("handle", &input)
        .expect("the guest handle export must return a JSON-RPC response");
    let response: ExtensionResponse = serde_json::from_slice(output).unwrap();
    assert_eq!(response.jsonrpc, JSONRPC_VERSION);
    assert_eq!(response.id, request.id);
    assert_ne!(response.result.is_some(), response.error.is_some());
    response
}

fn result(plugin: &mut Plugin, request: ExtensionRequest) -> Value {
    let response = call(plugin, request);
    assert!(response.error.is_none(), "{:?}", response.error);
    response.result.unwrap()
}

fn initialize(plugin: &mut Plugin) {
    let request = ExtensionRequest::new(
        methods::INITIALIZE,
        InitializeParams {
            protocol_versions: vec![MEP_VERSION.into()],
            host: PeerInfo {
                name: "rust-binding-integration-test".into(),
                version: "0.1.0".into(),
            },
        },
        1,
    )
    .unwrap();
    let initialized: InitializeResult = serde_json::from_value(result(plugin, request)).unwrap();
    assert_eq!(initialized.protocol_version, MEP_VERSION);
    assert_eq!(initialized.extension.id, "morphir-rust");
    assert_eq!(
        initialized.extension.types,
        vec![ExtensionType::Frontend, ExtensionType::Backend]
    );
    assert_eq!(
        initialized.capabilities.frontend.unwrap().ir_versions,
        ["3", "4"]
    );
    assert_eq!(
        initialized.capabilities.backend.unwrap().ir_versions,
        ["3", "4"]
    );
}

fn a_type_model(version: &str) -> CompileRequest {
    CompileRequest {
        language_id: "rust".into(),
        documents: vec![SourceDocument {
            uri: "models.rs".into(),
            language_id: "rust".into(),
            version: 1,
            text: "pub struct Person { pub name: String, pub age: i64 }\n\
                pub enum Decision { Pending, Accepted(Person) }"
                .into(),
        }],
        package: CompilePackage {
            name: "acme/example".into(),
            exposed_modules: vec!["Models".into()],
        },
        dependencies: vec![],
        options: CompileOptions {
            types_only: true,
            ir_version: version.into(),
            ..Default::default()
        },
    }
}

#[test]
#[ignore = "requires a built Rust WASM guest; see this file's module documentation"]
fn wasm_exports_compile_and_generate_both_ir_versions_like_native() {
    let mut plugin = guest();
    let metadata = plugin
        .call::<&[u8], &[u8]>("morphir_extension_info", &[])
        .unwrap();
    let metadata: ExtensionInfo = serde_json::from_slice(metadata).unwrap();
    assert_eq!(metadata.id, "morphir-rust");
    initialize(&mut plugin);

    for version in ["3", "4"] {
        let native = RustExtension.compile(a_type_model(version)).unwrap();
        assert!(native.success, "{:?}", native.diagnostics);
        let compiled: CompileResult = serde_json::from_value(result(
            &mut plugin,
            ExtensionRequest::new(methods::COMPILE, a_type_model(version), 2).unwrap(),
        ))
        .unwrap();
        assert!(compiled.success, "{:?}", compiled.diagnostics);
        assert_eq!(compiled.ir_version.as_deref(), Some(version));
        assert_eq!(compiled.ir, native.ir);
        assert_eq!(compiled.modules, native.modules);

        let request = GenerateRequest {
            ir: compiled.ir.unwrap(),
            target: "rust".into(),
            options: Default::default(),
        };
        let native = RustExtension.generate(request.clone()).unwrap();
        assert!(native.success, "{:?}", native.diagnostics);
        let generated: GenerateResult = serde_json::from_value(result(
            &mut plugin,
            ExtensionRequest::new(methods::GENERATE, request, 3).unwrap(),
        ))
        .unwrap();
        assert!(generated.success, "{:?}", generated.diagnostics);
        assert_eq!(generated.artifacts.len(), 1);
        assert_eq!(generated.artifacts[0].path, "lib.rs");
        assert!(!generated.artifacts[0].binary);
        assert_eq!(generated.artifacts[0].content, native.artifacts[0].content);
    }
}

#[test]
#[ignore = "requires a built Rust WASM guest; see this file's module documentation"]
fn wasm_returns_protocol_errors_and_remains_usable() {
    let mut plugin = guest();
    initialize(&mut plugin);
    for (method, params, expected) in [
        (methods::COMPILE, json!({}), error_codes::INVALID_PARAMS),
        (methods::GENERATE, json!({}), error_codes::INVALID_PARAMS),
        ("unknown.method", json!({}), error_codes::METHOD_NOT_FOUND),
    ] {
        let response = call(
            &mut plugin,
            ExtensionRequest::new(method, params, 41).unwrap(),
        );
        assert!(response.result.is_none());
        assert_eq!(response.error.unwrap().code, expected);
    }
    assert!(plugin.call::<&[u8], &[u8]>("handle", b"{").is_err());
    let pong = result(
        &mut plugin,
        ExtensionRequest::new(methods::PING, json!({}), 42).unwrap(),
    );
    assert_eq!(pong, json!({ "ok": true }));
}

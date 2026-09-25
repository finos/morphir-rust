//! Opt-in tests of the actual Extism guest exports.
//!
//! Run from the workspace root:
//! ```text
//! cargo +1.98.0 build -p morphir-elm-binding --target wasm32-unknown-unknown
//! cargo +1.98.0 test -p morphir-elm-binding --features wasm-host-tests --test wasm -- --ignored
//! ```
//! Set MORPHIR_ELM_GUEST to test a release or externally built guest instead.

// Gate the test body instead of the Cargo target so `--test '*'` works without
// the optional host dependency; the dedicated WASM job enables this feature.
#![cfg(all(feature = "wasm-host-tests", not(target_arch = "wasm32")))]

use extism::Plugin;
use morphir_elm_binding::ElmExtension;
use morphir_extension_sdk::{
    prelude::*,
    protocol::{
        ExtensionRequest, ExtensionResponse, InitializeParams, InitializeResult, JSONRPC_VERSION,
        MEP_VERSION, PeerInfo, error_codes, methods,
    },
};
use serde_json::{Value, json};
use std::path::PathBuf;

const EXAMPLE: &str =
    include_str!("../../morphir-host-native/tests/fixtures/morphir-elm-extension/Example.elm");
const TYPES: &str = include_str!("fixtures/Types.elm");
const OTHER: &str = "module My.Other exposing (Thing)\n\ntype Thing = Thing\n";

#[test]
#[ignore = "requires a built Elm WASM guest; see this file's module documentation"]
fn wasm_reports_its_metadata_and_compiles_the_example_module() {
    let mut plugin = guest();
    let metadata = plugin
        .call::<&[u8], &[u8]>("morphir_extension_info", &[])
        .unwrap();
    let metadata: ExtensionInfo = serde_json::from_slice(metadata).unwrap();
    assert_eq!(metadata.id, "morphir-elm-native");
    assert_eq!(metadata.name, "Morphir Elm (native)");
    initialize(&mut plugin);

    let compiled: CompileResult = serde_json::from_value(result(
        &mut plugin,
        ExtensionRequest::new(methods::COMPILE, an_example_request("3"), 2).unwrap(),
    ))
    .unwrap();
    assert!(compiled.success, "{:?}", compiled.diagnostics);
    assert_eq!(compiled.modules, vec!["Example".to_string()]);
    assert_eq!(compiled.ir_version.as_deref(), Some("3"));
    assert!(compiled.ir.is_some());
}

#[test]
#[ignore = "requires a built Elm WASM guest; see this file's module documentation"]
fn wasm_compiles_and_generates_both_ir_versions_like_native() {
    let mut plugin = guest();
    initialize(&mut plugin);
    for version in ["3", "4"] {
        let request = a_type_model(version);
        let native = ElmExtension.compile(request.clone()).unwrap();
        assert!(native.success, "{:?}", native.diagnostics);
        let compiled: CompileResult = serde_json::from_value(result(
            &mut plugin,
            ExtensionRequest::new(methods::COMPILE, request, 2).unwrap(),
        ))
        .unwrap();
        assert_eq!(
            serde_json::to_value(&compiled).unwrap(),
            serde_json::to_value(&native).unwrap()
        );

        let request = GenerateRequest {
            ir: compiled.ir.unwrap(),
            target: "elm".into(),
            options: Default::default(),
        };
        let native = ElmExtension.generate(request.clone()).unwrap();
        assert!(native.success, "{:?}", native.diagnostics);
        let generated: GenerateResult = serde_json::from_value(result(
            &mut plugin,
            ExtensionRequest::new(methods::GENERATE, request, 3).unwrap(),
        ))
        .unwrap();
        assert_eq!(
            serde_json::to_value(&generated).unwrap(),
            serde_json::to_value(&native).unwrap()
        );
    }
}

#[test]
#[ignore = "requires a built Elm WASM guest; see this file's module documentation"]
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

fn guest() -> Plugin {
    let path = std::env::var_os("MORPHIR_ELM_GUEST")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/wasm32-unknown-unknown/debug/morphir_elm_binding.wasm")
        });
    let wasm = std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "cannot read guest {}: {error}; build it with cargo +1.98.0 build \
             -p morphir-elm-binding --target wasm32-unknown-unknown, \
             or set MORPHIR_ELM_GUEST",
            path.display()
        )
    });
    // The binding requires neither WASI nor filesystem/network host functions.
    Plugin::new(wasm, [], false).expect("the Elm guest must load without host imports")
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
                kind: Default::default(),
                name: "elm-binding-integration-test".into(),
                version: "0.1.0".into(),
            },
        },
        1,
    )
    .unwrap();
    let initialized: InitializeResult = serde_json::from_value(result(plugin, request)).unwrap();
    assert_eq!(initialized.protocol_version, MEP_VERSION);
    assert_eq!(initialized.extension.id, "morphir-elm-native");
    assert_eq!(
        initialized.extension.types,
        vec![
            ExtensionType::Frontend,
            ExtensionType::Backend,
            ExtensionType::Workspace,
        ]
    );
    let frontend = initialized.capabilities.frontend.unwrap();
    assert_eq!(frontend.ir_versions, ["3", "4"]);
    assert_eq!(frontend.languages[0].id, "elm");
    assert!(frontend.incremental && !frontend.fragments);
    assert_eq!(
        initialized.capabilities.backend.unwrap().ir_versions,
        ["3", "4"]
    );
    let workspace = initialized.capabilities.workspace.unwrap();
    assert_eq!(
        workspace.protocol_versions,
        vec![morphir_workspace::workspace_discovery_protocol()]
    );
    assert!(workspace.discover);
}

fn request(documents: Vec<SourceDocument>, version: &str) -> CompileRequest {
    CompileRequest {
        language_id: "elm".into(),
        sources: SourceSet {
            root: None,
            documents,
        },
        package: CompilePackage {
            name: "local/example".into(),
            exposed_modules: Some(vec!["Example".into()]),
        },
        dependencies: vec![],
        options: CompileOptions {
            types_only: false,
            ir_version: version.into(),
            ..Default::default()
        },
        baseline: None,
    }
}

fn document(uri: &str, text: &str) -> SourceDocument {
    SourceDocument {
        uri: uri.into(),
        language_id: "elm".into(),
        version: 1,
        text: text.into(),
    }
}

fn an_example_request(version: &str) -> CompileRequest {
    request(vec![document("Example.elm", EXAMPLE)], version)
}

fn a_type_model(version: &str) -> CompileRequest {
    let mut request = request(
        vec![
            document("My/Domain/Types.elm", TYPES),
            document("My/Other.elm", OTHER),
        ],
        version,
    );
    request.package.exposed_modules = None;
    request
}

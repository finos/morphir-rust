//! Tests of the actual Gleam Extism guest and its native equivalent.
//!
//! Build with `cargo build -p morphir-gleam-binding --target wasm32-unknown-unknown`,
//! then run `cargo test -p morphir-gleam-binding --features wasm-host-tests --test wasm -- --ignored`.
//! Set MORPHIR_GLEAM_GUEST to select a packaged or release guest.
#![cfg(all(feature = "wasm-host-tests", not(target_arch = "wasm32")))]

use extism::Plugin;
use morphir_extension_sdk::{
    prelude::*,
    protocol::{
        ExtensionRequest, ExtensionResponse, InitializeParams, InitializeResult, JSONRPC_VERSION,
        MEP_VERSION, PeerInfo, error_codes, methods,
    },
};
use morphir_gleam_binding::GleamExtension;
use serde_json::{Value, json};
use std::path::PathBuf;

#[test]
#[ignore = "requires a built Gleam WASM guest"]
fn guest_metadata_and_initialization_match_native_capabilities() {
    let mut guest = guest();
    let metadata = guest
        .call::<&[u8], &[u8]>("morphir_extension_info", &[])
        .unwrap();
    let metadata: ExtensionInfo = serde_json::from_slice(metadata).unwrap();
    assert_eq!(metadata.id, "morphir-gleam");
    assert_eq!(
        serde_json::to_value(metadata).unwrap(),
        serde_json::to_value(GleamExtension::info()).unwrap()
    );
    initialize(&mut guest);
}

#[test]
#[ignore = "requires a built Gleam WASM guest"]
fn guest_compile_generate_and_incremental_match_native_for_both_versions() {
    let mut guest = guest();
    initialize(&mut guest);
    for version in ["3", "4"] {
        let mut request = request(version);
        let first = same_compilation(&mut guest, &request);
        let generate = GenerateRequest {
            ir: first.ir.clone().unwrap(),
            target: "gleam".into(),
            options: Default::default(),
        };
        let native = GleamExtension.generate(generate.clone()).unwrap();
        assert!(native.success, "{:?}", native.diagnostics);
        let generated: GenerateResult = serde_json::from_value(result(
            &mut guest,
            ExtensionRequest::new(methods::GENERATE, generate, 3).unwrap(),
        ))
        .unwrap();
        assert_eq!(
            serde_json::to_value(generated).unwrap(),
            serde_json::to_value(native).unwrap()
        );

        request.baseline = Some(baseline(&first));
        let reused = same_compilation(&mut guest, &request);
        assert!(
            reused
                .module_results
                .iter()
                .all(|module| module.status == ModuleStatus::Unchanged)
        );
        assert_eq!(first.ir, reused.ir);
        request.sources.documents[1].text = "pub type Number = Float\n".into();
        let changed = same_compilation(&mut guest, &request);
        assert!(
            changed
                .module_results
                .iter()
                .all(|module| module.status == ModuleStatus::Compiled)
        );
    }
}

#[test]
#[ignore = "requires a built Gleam WASM guest"]
fn guest_protocol_errors_do_not_poison_subsequent_requests() {
    let mut guest = guest();
    initialize(&mut guest);
    for (method, expected) in [
        (methods::COMPILE, error_codes::INVALID_PARAMS),
        (methods::GENERATE, error_codes::INVALID_PARAMS),
        ("unknown.method", error_codes::METHOD_NOT_FOUND),
    ] {
        let response = call(
            &mut guest,
            ExtensionRequest::new(method, json!({}), 41).unwrap(),
        );
        assert_eq!(response.error.unwrap().code, expected);
    }
    assert!(guest.call::<&[u8], &[u8]>("handle", b"{").is_err());
    assert_eq!(
        result(
            &mut guest,
            ExtensionRequest::new(methods::PING, json!({}), 42).unwrap()
        ),
        json!({"ok": true})
    );
}

#[test]
#[ignore = "requires a built Gleam WASM guest"]
fn guest_official_parser_and_diagnostics_match_native() {
    let mut guest = guest();
    initialize(&mut guest);
    for (source, succeeds) in [
        ("pub const answer: Int = 0x2A\n", true),
        ("pub fn text() { \"\\u{1F600}\" }\n", true),
        ("pub type Wrapped = (Int)\n", false),
        ("pub fn missing(x: Int) -> Int\n", false),
        ("pub const inferred = 42\n", false),
        (
            "pub fn labelled(named value: Int) -> Int { value }\n",
            false,
        ),
    ] {
        let mut request = request("4");
        request.options.types_only = false;
        request.sources.documents.truncate(1);
        request.sources.documents[0].text = source.into();
        let native = GleamExtension.compile(request.clone()).unwrap();
        assert_eq!(
            native.success, succeeds,
            "{source}: {:?}",
            native.diagnostics
        );
        let compiled: CompileResult = serde_json::from_value(result(
            &mut guest,
            ExtensionRequest::new(methods::COMPILE, request, 2).unwrap(),
        ))
        .unwrap();
        assert_eq!(
            serde_json::to_value(compiled).unwrap(),
            serde_json::to_value(native).unwrap(),
            "{source}"
        );
    }
}

fn same_compilation(guest: &mut Plugin, request: &CompileRequest) -> CompileResult {
    let native = GleamExtension.compile(request.clone()).unwrap();
    assert!(native.success, "{:?}", native.diagnostics);
    let compiled: CompileResult = serde_json::from_value(result(
        guest,
        ExtensionRequest::new(methods::COMPILE, request, 2).unwrap(),
    ))
    .unwrap();
    assert_eq!(
        serde_json::to_value(&compiled).unwrap(),
        serde_json::to_value(native).unwrap()
    );
    compiled
}

fn baseline(result: &CompileResult) -> CompileBaseline {
    CompileBaseline {
        context_digest: result.context_digest.clone(),
        modules: result
            .module_results
            .iter()
            .map(|module| BaselineModule {
                frontend_state: module.frontend_state.clone(),
                name: module.name.clone(),
                uri: module.uri.clone(),
                source_digest: module.source_digest.clone().unwrap(),
                interface_digest: module.interface_digest.clone().unwrap(),
                depends_on: module.depends_on.clone(),
                ir: module.ir.clone().unwrap(),
            })
            .collect(),
    }
}

fn request(version: &str) -> CompileRequest {
    CompileRequest {
        language_id: "gleam".into(),
        sources: SourceSet {
            root: None,
            documents: [
                (
                    "model",
                    "import numbers\npub type Amount = numbers.Number\npub type Outcome(a) { Pending Success(value: a, amount: Amount) Failed(String) Retry(Outcome(a)) }\n",
                ),
                ("numbers", "pub type Number = Int\n"),
            ]
            .into_iter()
            .map(|(name, source)| SourceDocument {
                uri: format!("file:///src/{name}.gleam"),
                language_id: "gleam".into(),
                version: 1,
                text: source.into(),
            })
            .collect(),
        },
        package: CompilePackage {
            name: "sample".into(),
            exposed_modules: None,
        },
        options: CompileOptions {
            ir_version: version.into(),
            types_only: true,
            extra: [("emitParseStage".into(), false.into())].into(),
        },
        ..Default::default()
    }
}

fn guest() -> Plugin {
    let path = std::env::var_os("MORPHIR_GLEAM_GUEST")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/wasm32-unknown-unknown/debug/morphir_gleam_binding.wasm")
        });
    let wasm = std::fs::read(&path).unwrap_or_else(|error| panic!(
        "cannot read guest {}: {error}; build the wasm32-unknown-unknown target or set MORPHIR_GLEAM_GUEST", path.display()
    ));
    Plugin::new(wasm, [], false).expect("Gleam guest loads without WASI or host functions")
}

fn call(guest: &mut Plugin, request: ExtensionRequest) -> ExtensionResponse {
    let input = serde_json::to_vec(&request).unwrap();
    let output = guest.call::<&[u8], &[u8]>("handle", &input).unwrap();
    let response: ExtensionResponse = serde_json::from_slice(output).unwrap();
    assert_eq!(response.jsonrpc, JSONRPC_VERSION);
    assert_eq!(response.id, request.id);
    assert_ne!(response.result.is_some(), response.error.is_some());
    response
}

fn result(guest: &mut Plugin, request: ExtensionRequest) -> Value {
    let response = call(guest, request);
    assert!(response.error.is_none(), "{:?}", response.error);
    response.result.unwrap()
}

fn initialize(guest: &mut Plugin) {
    let initialized: InitializeResult = serde_json::from_value(result(
        guest,
        ExtensionRequest::new(
            methods::INITIALIZE,
            InitializeParams {
                protocol_versions: vec![MEP_VERSION.into()],
                host: PeerInfo {
                    name: "gleam-wasm-test".into(),
                    version: "0.1.0".into(),
                },
            },
            1,
        )
        .unwrap(),
    ))
    .unwrap();
    assert_eq!(initialized.protocol_version, MEP_VERSION);
    assert_eq!(initialized.extension.id, "morphir-gleam");
    let capabilities = initialized.capabilities;
    assert_eq!(
        serde_json::to_value(&capabilities).unwrap(),
        serde_json::to_value(GleamExtension::capabilities()).unwrap()
    );
    assert!(capabilities.frontend.unwrap().incremental);
}

#[test]
#[ignore = "requires a built Gleam WASM guest"]
fn guest_structural_values_match_native_and_the_golden() {
    let mut guest = guest();
    initialize(&mut guest);
    let mut request = request("4");
    request.options.types_only = false;
    request.sources.documents.truncate(1);
    request.sources.documents[0].text = include_str!("fixtures/structural_values.gleam").into();
    let compiled = same_compilation(&mut guest, &request);
    let generate = GenerateRequest {
        ir: compiled.ir.unwrap(),
        target: "gleam".into(),
        options: Default::default(),
    };
    let native = GleamExtension.generate(generate.clone()).unwrap();
    assert!(native.success, "{:?}", native.diagnostics);
    assert_eq!(native.artifacts.len(), 1);
    assert_eq!(
        native.artifacts[0].content,
        include_str!("goldens/source_structural_values.gleam")
    );
    let generated: GenerateResult = serde_json::from_value(result(
        &mut guest,
        ExtensionRequest::new(methods::GENERATE, generate, 3).unwrap(),
    ))
    .unwrap();
    assert_eq!(
        serde_json::to_value(generated).unwrap(),
        serde_json::to_value(native).unwrap()
    );
}

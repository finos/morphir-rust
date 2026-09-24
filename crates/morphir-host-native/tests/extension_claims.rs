//! CLI conformance for claims read from exact WASM bytes.

use morphir_avro_extension::AvroExtension;
use morphir_extension_sdk::Extension;
use morphir_extension_sdk::claims::CapabilityClaimSet;
use morphir_extension_sdk::protocol::SUPPORTED_MEP_VERSIONS;
use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Output};

fn run_claims(path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_extension-claims"))
        .arg(path)
        .output()
        .expect("claims tool should start")
}

fn assert_failure(output: Output, message: &str) {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(message), "unexpected stderr: {stderr}");
    assert!(!stderr.contains("panicked"), "unexpected panic: {stderr}");
}

fn avro_claims() -> Value {
    serde_json::to_value(
        CapabilityClaimSet::from_metadata(
            SUPPORTED_MEP_VERSIONS.iter().map(|v| (*v).into()).collect(),
            AvroExtension::info(),
            &AvroExtension::capabilities(),
        )
        .unwrap(),
    )
    .unwrap()
}

/// A minimal Extism guest whose discovery export succeeds and whose handle
/// returns the specified JSON-RPC envelope, or has no handle export at all.
fn a_guest_answering(response: Option<Value>) -> Vec<u8> {
    fn export(name: &str, text: &str) -> String {
        let writes = text
            .bytes()
            .enumerate()
            .map(|(index, byte)| {
                format!(
                    "(call $store_u8 (i64.add (local.get $out) (i64.const {index})) (i32.const {byte}))"
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            r#"(func (export "{name}") (result i32)
                (local $out i64)
                (local.set $out (call $alloc (i64.const {length})))
                {writes}
                (call $output_set (local.get $out) (i64.const {length}))
                (i32.const 0))"#,
            length = text.len(),
        )
    }
    let info = export(
        "morphir_extension_info",
        &serde_json::to_string(&AvroExtension::info()).unwrap(),
    );
    let handle = response
        .map(|value| export("handle", &value.to_string()))
        .unwrap_or_default();
    wat::parse_str(format!(
        r#"(module
            (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
            (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
            (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
            {info}
            {handle})"#
    ))
    .unwrap()
}

fn run_fixture(bytes: &[u8]) -> Output {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("guest.wasm");
    std::fs::write(&path, bytes).unwrap();
    run_claims(&path)
}

#[test]
fn prints_only_the_claim_set() {
    let claims = avro_claims();
    let output = run_fixture(&a_guest_answering(Some(json!({
        "jsonrpc": "2.0", "id": 1, "result": claims
    }))));
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        claims
    );
}

#[test]
fn rejects_a_file_that_is_not_a_guest() {
    assert_failure(run_fixture(b"not a WASM guest"), "extension-claims:");
    assert_failure(
        run_fixture(&wat::parse_str("(module)").unwrap()),
        "extension-claims:",
    );
}

#[test]
fn rejects_a_guest_without_a_handler() {
    assert_failure(run_fixture(&a_guest_answering(None)), "describe");
}

#[test]
fn reports_describe_rpc_errors() {
    for (code, message) in [(-32601, "Method not found"), (-32603, "describe failed")] {
        let guest = a_guest_answering(Some(json!({
            "jsonrpc": "2.0", "id": 1,
            "error": {"code": code, "message": message}
        })));
        assert_failure(run_fixture(&guest), message);
    }
}

#[test]
fn rejects_malformed_responses() {
    for response in [
        json!({"jsonrpc": "2.0", "id": 2, "result": avro_claims()}),
        json!({"jsonrpc": "2.0", "id": 1, "result": {}}),
        json!({"jsonrpc": "2.0", "id": 1}),
    ] {
        assert_failure(run_fixture(&a_guest_answering(Some(response))), "describe");
    }
}

#[test]
fn requires_exactly_one_path() {
    for args in [vec![], vec!["one.wasm", "two.wasm"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_extension-claims"))
            .args(args)
            .output()
            .unwrap();
        assert_failure(output, "Usage:");
    }
}

#[test]
#[ignore = "requires cargo build --locked --release -p morphir-avro-extension --target wasm32-unknown-unknown"]
fn avro_wasm_claims_equal_native_metadata() {
    let metadata = Command::new(env!("CARGO"))
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .arg("--manifest-path")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"))
        .output()
        .unwrap();
    assert!(metadata.status.success());
    let metadata: Value = serde_json::from_slice(&metadata.stdout).unwrap();
    let path = Path::new(metadata["target_directory"].as_str().unwrap())
        .join("wasm32-unknown-unknown/release/morphir_avro_extension.wasm");
    assert!(
        path.is_file(),
        "build the Avro guest first: {}",
        path.display()
    );
    let output = run_claims(&path);
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        avro_claims()
    );
}

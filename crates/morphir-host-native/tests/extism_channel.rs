//! `ExtismChannel` driven through a full session against a small WAT guest.

use morphir_extension_sdk::protocol::{ExtensionResponse, InitializeResult, PeerInfo, PeerKind};
use morphir_extension_sdk::{
    BackendCapability, ExtensionCapabilities, ExtensionInfo, ExtensionType, GenerateRequest,
};
use morphir_host::{
    ExpectedChecks, ExpectedExtension, HostConfig, JsonRpcConnection,
    PersistedExtensionCapabilities, Session,
};
use morphir_host_native::CheckedConnection;
use morphir_host_native::extism::{ExtensionContainer, ExtismChannel, MorphirHostFunctions};
use serde_json::json;

fn config() -> HostConfig {
    HostConfig::new(PeerInfo {
        kind: PeerKind::Unspecified,
        name: "test".into(),
        version: "1.0.0".into(),
    })
}

fn info() -> ExtensionInfo {
    ExtensionInfo {
        id: "wat-backend".into(),
        name: "WAT Backend".into(),
        version: "1.0.0".into(),
        types: vec![ExtensionType::Backend],
        ..ExtensionInfo::default()
    }
}

fn backend() -> BackendCapability {
    BackendCapability {
        targets: vec!["x".into()],
        ir_versions: vec!["4".into()],
        generate: true,
    }
}

/// WAT instructions that write `text` to guest memory at `$output`.
fn writes(text: &str) -> String {
    text.bytes()
        .enumerate()
        .map(|(index, byte)| {
            format!(
                "(call $store_u8 (i64.add (local.get $output) (i64.const {index})) (i32.const {byte}))"
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// WAT instructions that set the call's output to `text`.
fn output(text: &str) -> String {
    format!(
        r#"(local.set $output (call $alloc (i64.const {length})))
            {writes}
            (call $output_set (local.get $output) (i64.const {length}))"#,
        length = text.len(),
        writes = writes(text),
    )
}

/// A guest whose `handle` export answers its nth call with `responses[n]`.
///
/// Extism keeps the instance between calls, so a global counts the calls.
fn scripted_guest(info: &ExtensionInfo, responses: &[ExtensionResponse]) -> Vec<u8> {
    let branches = responses
        .iter()
        .enumerate()
        .map(|(index, response)| {
            let text = serde_json::to_string(response).unwrap();
            format!(
                "(if (i32.eq (global.get $calls) (i32.const {index})) (then {}))",
                output(&text)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    wat::parse_str(format!(
        r#"(module
            (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
            (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
            (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
            (global $calls (mut i32) (i32.const 0))
            (func (export "morphir_extension_info") (result i32)
                (local $output i64)
                {info}
                (i32.const 0))
            (func (export "handle") (result i32)
                (local $output i64)
                {branches}
                (global.set $calls (i32.add (global.get $calls) (i32.const 1)))
                (i32.const 0)))"#,
        info = output(&serde_json::to_string(info).unwrap()),
    ))
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wasm_backend_generates_through_a_checked_session() {
    let initialized = InitializeResult {
        protocol_version: "0.1".into(),
        extension: info(),
        capabilities: ExtensionCapabilities {
            backend: Some(backend()),
            ..ExtensionCapabilities::default()
        },
    };
    let guest = scripted_guest(
        &info(),
        &[
            ExtensionResponse::success(1, initialized).unwrap(),
            ExtensionResponse::success(
                2,
                json!({
                    "success": true,
                    "artifacts": [{"path": "out/Main.x", "content": "main"}],
                    "diagnostics": []
                }),
            )
            .unwrap(),
            ExtensionResponse::success(3, json!({})).unwrap(),
        ],
    );
    let working_directory = tempfile::tempdir().unwrap();
    let container = ExtensionContainer::from_bytes_async(
        info().id,
        guest,
        MorphirHostFunctions::for_restricted_generation(working_directory.path().to_path_buf()),
    )
    .await
    .unwrap();
    assert_eq!(container.info().id, "wat-backend");
    let expected = ExpectedExtension::discovered_with_persisted_capabilities(
        info(),
        PersistedExtensionCapabilities::new(None, Some(backend())),
    );
    let channel = ExtismChannel::new(container, expected);
    let checks = ExpectedChecks::new(channel.expectation());
    let connection = CheckedConnection::new(JsonRpcConnection::new(channel, checks));

    let mut session = Session::open(connection, &config()).await.unwrap();
    assert_eq!(session.negotiated().extension().id, "wat-backend");
    let result = session
        .generate(GenerateRequest {
            ir: json!({}),
            target: "x".into(),
            options: Default::default(),
        })
        .await
        .unwrap();
    session.close().await.unwrap();

    assert!(result.success);
    assert_eq!(result.artifacts.len(), 1);
    assert_eq!(result.artifacts[0].path, "out/Main.x");
}

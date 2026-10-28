//! The restricted host profile applies to actual Extism instances.

use morphir_host_native::extism::{
    ExtensionContainer, MorphirHostFunctions, RestrictedExtismPolicy,
};
use std::time::Duration;

fn a_policy(request_bytes: u64, response_bytes: u64) -> RestrictedExtismPolicy {
    RestrictedExtismPolicy::new(
        256 * 1024 * 1024,
        1_000_000_000,
        Duration::from_secs(30),
        request_bytes,
        response_bytes,
    )
    .unwrap()
}

fn writes(text: &str) -> String {
    text.bytes()
        .enumerate()
        .map(|(i, b)| {
            format!("(call $store (i64.add (local.get $out) (i64.const {i})) (i32.const {b}))")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

const INFO: &str =
    r#"{"id":"restricted-fixture","name":"Restricted","version":"1.0.0","types":["backend"]}"#;

fn a_guest(info: &str, imports: &str, handle: &str) -> Vec<u8> {
    wat::parse_str(format!(
        r#"(module
          (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
          (import "extism:host/env" "store_u8" (func $store (param i64 i32)))
          (import "extism:host/env" "output_set" (func $output (param i64 i64)))
          {imports}
          (func (export "morphir_extension_info") (result i32)
            (local $out i64)
            (local.set $out (call $alloc (i64.const {info_len})))
            {info_writes}
            (call $output (local.get $out) (i64.const {info_len}))
            (i32.const 0))
          (func (export "handle") (result i32)
            {handle}))"#,
        info_len = info.len(),
        info_writes = writes(info),
    ))
    .unwrap()
}

fn a_guest_with_output(bytes: usize) -> Vec<u8> {
    a_guest(
        INFO,
        "",
        &format!(
            r#"(local $out i64)
        (local.set $out (call $alloc (i64.const {bytes})))
        (call $output (local.get $out) (i64.const {bytes}))
        (i32.const 0)"#
        ),
    )
}

#[tokio::test]
async fn bounds_actual_requests_and_outputs_at_the_exact_policy_limit() {
    let guest = a_guest_with_output(1024);
    let host = ExtensionContainer::from_bytes_with_execution_policy(
        "restricted-fixture",
        &guest,
        MorphirHostFunctions::default(),
        a_policy(512, 1024),
    )
    .unwrap();
    assert_eq!(
        host.call_raw("handle", &[0; 512]).await.unwrap().len(),
        1024
    );
    assert!(
        host.call_raw("handle", &[0; 513])
            .await
            .unwrap_err()
            .to_string()
            .contains("request exceeds")
    );
    assert!(
        host.call::<_, serde_json::Value>("test", "x".repeat(512))
            .await
            .unwrap_err()
            .to_string()
            .contains("request exceeds")
    );
    let oversized = ExtensionContainer::from_bytes_with_execution_policy(
        "restricted-fixture",
        &a_guest_with_output(1025),
        MorphirHostFunctions::default(),
        a_policy(512, 1024),
    )
    .unwrap();
    assert!(
        oversized
            .call_raw("handle", &[])
            .await
            .unwrap_err()
            .to_string()
            .contains("response exceeds")
    );
}

#[test]
fn bounds_metadata_discovery_at_the_exact_policy_limit() {
    let guest = a_guest_with_output(0);
    ExtensionContainer::from_bytes_with_execution_policy(
        "restricted-fixture",
        &guest,
        MorphirHostFunctions::default(),
        a_policy(512, INFO.len() as u64),
    )
    .unwrap();
    let error = ExtensionContainer::from_bytes_with_execution_policy(
        "restricted-fixture",
        &guest,
        MorphirHostFunctions::default(),
        a_policy(512, INFO.len() as u64 - 1),
    )
    .err()
    .expect("metadata above the policy must fail");
    assert!(error.to_string().contains("response exceeds"));
}

#[test]
fn refuses_zero_unaligned_or_expanded_policy_limits() {
    let valid = [
        256 * 1024 * 1024,
        1_000_000_000,
        64 * 1024 * 1024,
        64 * 1024 * 1024,
    ];
    for position in 0..valid.len() {
        for invalid in [0, valid[position] + 1] {
            let mut bounds = valid;
            bounds[position] = invalid;
            assert!(
                RestrictedExtismPolicy::new(
                    bounds[0],
                    bounds[1],
                    Duration::from_secs(30),
                    bounds[2],
                    bounds[3]
                )
                .is_err()
            );
        }
    }
    for timeout in [
        Duration::ZERO,
        Duration::from_nanos(1),
        Duration::from_secs(30) + Duration::from_nanos(1),
    ] {
        assert!(
            RestrictedExtismPolicy::new(valid[0], valid[1], timeout, valid[2], valid[3]).is_err()
        );
    }
    assert!(
        RestrictedExtismPolicy::new(65537, valid[1], Duration::from_secs(30), valid[2], valid[3])
            .is_err()
    );
}

#[tokio::test]
async fn denies_http_before_contacting_a_real_local_listener() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let request = format!(
        r#"{{"url":"http://{}/secret"}}"#,
        listener.local_addr().unwrap()
    );
    let guest = a_guest(
        INFO,
        r#"(import "extism:host/env" "http_request" (func $http (param i64 i64) (result i64)))"#,
        &format!(
            r#"(local $out i64)
        (local.set $out (call $alloc (i64.const {len})))
        {write_request}
        (drop (call $http (local.get $out) (i64.const 0)))
        unreachable (i32.const 0)"#,
            len = request.len(),
            write_request = writes(&request)
        ),
    );
    let host = ExtensionContainer::from_bytes_with_execution_policy(
        "restricted-fixture",
        &guest,
        MorphirHostFunctions::default(),
        a_policy(512, 1024),
    )
    .unwrap();
    let error = host.call_raw("handle", &[]).await.unwrap_err().to_string();
    assert!(error.contains("not allowed"), "{error}");
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn denies_filesystem_access_and_inherits_no_guest_environment() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("secret"), b"private").unwrap();
    let guest = a_guest(
        INFO,
        r#"
        (import "wasi_snapshot_preview1" "environ_sizes_get" (func $env (param i32 i32) (result i32)))
        (import "wasi_snapshot_preview1" "path_open" (func $open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 16) "secret")"#,
        r#"(if (call $env (i32.const 0) (i32.const 4)) (then unreachable))
        (if (i32.or (i32.load (i32.const 0)) (i32.load (i32.const 4))) (then unreachable))
        (if (i32.ne (call $open (i32.const 3) (i32.const 0) (i32.const 16) (i32.const 6)
          (i32.const 0) (i64.const 2) (i64.const 0) (i32.const 0) (i32.const 8)) (i32.const 8))
          (then unreachable))
        (if (i32.ne (call $open (i32.const 3) (i32.const 0) (i32.const 16) (i32.const 6)
          (i32.const 1) (i64.const 64) (i64.const 0) (i32.const 0) (i32.const 8)) (i32.const 8))
          (then unreachable))
        (i32.const 0)"#,
    );
    // A nonempty workspace path supplied to the Morphir callbacks is not a WASI grant.
    let host = ExtensionContainer::from_bytes_with_execution_policy(
        "restricted-fixture",
        &guest,
        MorphirHostFunctions::for_workspace(
            directory.path().to_owned(),
            directory.path().to_owned(),
        ),
        a_policy(512, 1024),
    )
    .unwrap();
    host.call_raw("handle", &[]).await.unwrap();
    assert_eq!(
        std::fs::read(directory.path().join("secret")).unwrap(),
        b"private"
    );
}

#[tokio::test]
async fn renews_the_same_finite_fuel_budget_for_each_call_after_discovery() {
    let guest = a_guest(
        INFO,
        "",
        r#"
        (local $remaining i32)
        (local.set $remaining (i32.const 1000))
        (loop $again
          (local.set $remaining (i32.sub (local.get $remaining) (i32.const 1)))
          (br_if $again (local.get $remaining)))
        (i32.const 0)"#,
    );
    let policy = RestrictedExtismPolicy::new(
        256 * 1024 * 1024,
        10_000,
        Duration::from_secs(30),
        512,
        1024,
    )
    .unwrap();
    let host = ExtensionContainer::from_bytes_with_execution_policy(
        "restricted-fixture",
        &guest,
        MorphirHostFunctions::default(),
        policy,
    )
    .unwrap();
    // The calls' aggregate work exceeds one budget. Every independent call fits.
    for _ in 0..50 {
        host.call_raw("handle", &[]).await.unwrap();
    }
    let looping = a_guest(INFO, "", r#"(loop $forever (br $forever)) (i32.const 0)"#);
    let host = ExtensionContainer::from_bytes_with_execution_policy(
        "restricted-fixture",
        &looping,
        MorphirHostFunctions::default(),
        policy,
    )
    .unwrap();
    assert!(
        host.call_raw("handle", &[])
            .await
            .unwrap_err()
            .to_string()
            .contains("fuel")
    );
}

#[tokio::test]
async fn applies_timeouts_to_async_discovery_and_operations() {
    let policy = RestrictedExtismPolicy::new(
        256 * 1024 * 1024,
        1_000_000_000,
        Duration::from_millis(10),
        512,
        1024,
    )
    .unwrap();
    let discovery = wat::parse_str(
        r#"(module
        (func (export "morphir_extension_info") (result i32)
          (loop $forever (br $forever)) (i32.const 0)))"#,
    )
    .unwrap();
    let began = std::time::Instant::now();
    let error = ExtensionContainer::from_bytes_with_execution_policy_async(
        "restricted-fixture".into(),
        discovery,
        MorphirHostFunctions::default(),
        policy,
    )
    .await
    .err()
    .expect("discovery loop must end")
    .to_string();
    assert!(error.contains("timeout"), "{error}");
    assert!(began.elapsed() < Duration::from_secs(3));
    let host = ExtensionContainer::from_bytes_with_execution_policy_async(
        "restricted-fixture".into(),
        a_guest(INFO, "", r#"(loop $forever (br $forever)) (i32.const 0)"#),
        MorphirHostFunctions::default(),
        policy,
    )
    .await
    .unwrap();
    assert!(
        host.call_raw("handle", &[])
            .await
            .unwrap_err()
            .to_string()
            .contains("timeout")
    );
}

#[tokio::test]
async fn applies_a_tighter_memory_ceiling_to_the_actual_plugin() {
    let guest = a_guest(
        INFO,
        "(memory 1)",
        r#"
        (if (i32.ne (memory.grow (i32.const 257)) (i32.const -1)) (then unreachable))
        (i32.const 0)"#,
    );
    let policy = RestrictedExtismPolicy::new(
        16 * 1024 * 1024,
        1_000_000_000,
        Duration::from_secs(30),
        512,
        1024,
    )
    .unwrap();
    let host = ExtensionContainer::from_bytes_with_execution_policy(
        "restricted-fixture",
        &guest,
        MorphirHostFunctions::default(),
        policy,
    )
    .unwrap();
    assert!(
        host.call_raw("handle", &[])
            .await
            .unwrap_err()
            .to_string()
            .contains("oom")
    );
}

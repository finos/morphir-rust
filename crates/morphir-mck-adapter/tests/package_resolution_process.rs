use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn exchange(args: &[&str], text: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mck-adapter-rust"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn draft_two_capabilities_are_exact() {
    let output = exchange(
        &["--suite", "package", "--contract", "0.1.0-draft.2"],
        "{\"id\":1,\"op\":\"capabilities\"}\n{\"id\":2,\"op\":\"exit\"}\n",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value,
        json!({
            "id": 1,
            "suite": "package",
            "contractVersion": "0.1.0-draft.2",
            "implementation": "morphir-rust",
            "implementationVersion": env!("CARGO_PKG_VERSION"),
            "operations": ["resolve-library"],
            "profiles": ["flat-library"]
        })
    );
}

#[test]
fn resolve_library_returns_domain_rejections_without_failing_the_process() {
    let request = json!({"id":1,"op":"resolve-library","input":"{"});
    let output = exchange(
        &["--suite", "package", "--contract", "0.1.0-draft.2"],
        &format!("{request}\n"),
    );
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["id"], 1);
    assert_eq!(value["ok"], false);
    assert_eq!(value["diagnostic"]["code"], "invalid-input");
}

#[test]
fn selectors_are_strict_and_checked_before_reading_requests() {
    for args in [
        vec!["--contract", "0.1.0-draft.2", "--suite", "package"],
        vec!["--suite", "package", "--suite", "package"],
        vec!["--suite", "package", "--contract", "unknown"],
        vec!["--suite", "ir", "--contract", "0.1.0-draft.2"],
    ] {
        let output = exchange(&args, "{\"id\":1,\"op\":\"capabilities\"}\n");
        assert!(!output.status.success(), "accepted {args:?}");
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn empty_protocol_input_fails_without_writing_stdout() {
    let output = exchange(&["--suite", "package", "--contract", "0.1.0-draft.2"], "");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}

#[test]
fn invalid_envelopes_and_unsupported_operations_fail_the_process() {
    for request in [
        r#"{"id":1,"op":"capabilities","extra":true}"#,
        r#"{"id":1,"op":"unknown"}"#,
        r#"{"id":0,"op":"capabilities"}"#,
        r#"{"id":1,"op":"resolve-library"}"#,
    ] {
        let output = exchange(
            &["--suite", "package", "--contract", "0.1.0-draft.2"],
            &format!("{request}\n"),
        );
        assert!(!output.status.success(), "accepted {request}");
        assert!(output.stdout.is_empty(), "wrote stdout for {request}");
        assert!(!output.stderr.is_empty(), "wrote no error for {request}");
    }
}

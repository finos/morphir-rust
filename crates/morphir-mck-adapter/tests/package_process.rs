use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn exchange(text: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mck-adapter-rust"))
        .args(["--suite", "package"])
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
fn package_capabilities_and_clean_exit() {
    let output = exchange("{\"id\":1,\"op\":\"capabilities\"}\n{\"id\":2,\"op\":\"exit\"}\n");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["suite"], "package");
    assert_eq!(response["contractVersion"], "0.1.0-draft.1");
    assert_eq!(response["operations"].as_array().unwrap().len(), 4);
}

#[test]
fn positive_integer_ids_follow_json_numeric_value() {
    for id in [
        "1.0",
        "1e2",
        "18446744073709551616",
        "1e400",
        "1000e-3",
        "1.200e1",
        "0.0001e4",
        "10e-00001",
        "1e+00000",
        "1e999999999999999999999999999999999999999999999",
    ] {
        let output = exchange(&format!("{{\"id\":{id},\"op\":\"capabilities\"}}\n"));
        assert!(output.status.success(), "id {id}: {:?}", output);
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["suite"], "package");
    }
}

#[test]
fn fractional_ids_are_rejected_without_rounding() {
    for id in [
        "0.99999999999999999",
        "1.0000000000000001",
        "9007199254740992.1",
        "1000e-4",
        "1.2e0",
        "1.200e-1",
        "0e999999999999999999999999",
        "-0.0",
        "1e-999999999999999999999999999999999999999999999",
    ] {
        let output = exchange(&format!("{{\"id\":{id},\"op\":\"capabilities\"}}\n"));
        assert!(
            !output.status.success(),
            "accepted nonpositive or fractional id {id}"
        );
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn canonical_metadata_and_exact_bytes() {
    let output = exchange(&format!(
        "{}\n{}\n",
        json!({"id":1,"op":"normalize","input":r#"{"2":"b","10":"a"}"#}),
        json!({"id":2,"op":"hash-bytes","hex":"616263"})
    ));
    assert!(output.status.success(), "{:?}", output);
    let lines: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines[0]["canonical"], r#"{"10":"a","2":"b"}"#);
    assert_eq!(
        lines[1]["digest"],
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn malformed_protocol_is_a_process_failure() {
    for text in [
        r#"{"id":0,"op":"capabilities"}"#,
        r#"{"id":-1,"op":"capabilities"}"#,
        r#"{"id":1.5,"op":"capabilities"}"#,
        r#"{"id":"1","op":"capabilities"}"#,
        r#"{"id":1,"op":"unknown"}"#,
        r#"{"id":1,"op":"capabilities","extra":true}"#,
        r#"{"id":1,"op":"hash-bytes","hex":"AB"}"#,
        r#"{"id":1,"op":"hash-bytes","hex":"a"}"#,
        r#"{"id":1,"op":"capabilities","\u006fp":"exit"}"#,
    ] {
        let output = exchange(&format!("{text}\n"));
        assert!(!output.status.success(), "accepted {text}");
        assert!(
            output.stdout.is_empty(),
            "protocol failure must not return document result"
        );
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn malformed_schema_is_infrastructure_failure() {
    for schema in [
        json!({"type":42}),
        json!({"$ref":"https://example.invalid/schema"}),
        json!({"$ref":"file:///etc/passwd"}),
        json!({"$ref":"#/$defs/missing"}),
    ] {
        let request = json!({"id":1,"op":"validate","artifact":"manifest","input":"{}","schemas":{"manifest":schema,"lock":{}}});
        let output = exchange(&format!("{request}\n"));
        assert!(!output.status.success(), "accepted {schema}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn invalid_metadata_is_a_document_result() {
    for input in [
        r#"{"a":"x","\u0061":"y"}"#,
        "123",
        "null",
        "true",
        "\u{feff}{}",
        r#""\n""#,
        r#"{"x":[{"a":"1","a":"2"}]}"#,
    ] {
        let output = exchange(&format!(
            "{}\n",
            json!({"id":1,"op":"normalize","input":input})
        ));
        assert!(output.status.success());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            value,
            json!({"id":1,"ok":false,"error":"invalid-document"}),
            "{input}"
        );
    }
}

fn a_library_request() -> Value {
    use morphir_package::{digest::Digest, metadata::NormalizedMetadata};
    let ir = json!({"formatVersion":4,"distribution":{"Library":{"packageName":"example/library","dependencies":{},"def":{"modules":{"api":{"Public":{"types":{},"values":{}}}}}}}}).to_string();
    let manifest = json!({"formatVersion":"0.1.0-draft.1","kind":"Library","packagePath":"example.com/library","version":"1.0.0","ir":{"formatVersion":"4","packageName":"example/library","payload":{"path":"ir.json","mediaType":"application/json","profile":"classic"}},"dependencies":{},"exports":{"api":"api"},"content":{"ir.json":Digest::of_bytes(ir.as_bytes()).to_string()}}).to_string();
    let metadata = NormalizedMetadata::parse(&manifest).unwrap();
    let lock = json!({"formatVersion":"0.1.0-draft.1","kind":"LibraryLockCore","root":"n0","nodes":{"n0":{"release":{"packagePath":"example.com/library","version":"1.0.0"},"irPackageName":"example/library","manifestDigest":metadata.manifest_digest().to_string(),"contentDigest":metadata.content_digest().to_string(),"bindings":{}}}}).to_string();
    json!({"id":1,"op":"verify-library-set","lock":lock,"libraries":[{"manifest":manifest,"files":[{"path":"ir.json","hex":ir.bytes().map(|b|format!("{b:02x}")).collect::<String>()}]}],"schemas":{"manifest":{"type":"object"},"lock":{"type":"object"}}})
}

#[test]
fn library_verification_checks_content_graph_and_ir() {
    let request = a_library_request();
    let output = exchange(&format!("{request}\n"));
    assert!(output.status.success(), "{:?}", output);
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["valid"], true);
    let mut invalid = request.clone();
    invalid["libraries"][0]["files"][0]["hex"] = json!("7b7d");
    let output = exchange(&format!("{invalid}\n"));
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["valid"],
        false
    );
    let mut invalid = request;
    let mut lock: Value = serde_json::from_str(invalid["lock"].as_str().unwrap()).unwrap();
    lock["root"] = json!("n9");
    invalid["lock"] = json!(lock.to_string());
    let output = exchange(&format!("{invalid}\n"));
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["valid"],
        false
    );
}

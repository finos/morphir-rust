use morphir_mck_adapter::protocol::*;
use morphir_mck_adapter::runtime::run;

#[test]
fn a_decode_request_parses_and_unknown_fields_are_refused() {
    let (id, req) = parse_line(r#"{"id":2,"op":"decode","version":4,"profile":"json","path":"current","strip":true,"node":"Type","input":"\"a\""}"#).unwrap();
    assert_eq!(id, 2);
    assert!(matches!(req, Request::Decode(d) if d.node == NodeKind::Type && d.strip));
    let (id, diag) = parse_line(r#"{"id":3,"op":"decode","version":4,"profile":"json","path":"current","strip":true,"node":"Type","input":"","extra":1}"#).unwrap_err();
    assert_eq!(id, Some(3));
    assert_eq!(diag.code, "protocol_error");
}

#[test]
fn malformed_lines_report_invalid_json_without_an_id() {
    let (id, diag) = parse_line("{not json").unwrap_err();
    assert_eq!(id, None);
    assert_eq!(diag.code, "protocol_error");
}

#[test]
fn capabilities_match_the_stage_one_contract() {
    let caps = serde_json::to_value(capabilities()).unwrap();
    assert_eq!(caps["contractVersion"], 1);
    assert_eq!(caps["binding"], "morphir-rust");
    assert_eq!(caps["language"], "rust");
    assert_eq!(caps["versions"], serde_json::json!([3, 4]));
    assert_eq!(caps["profiles"], serde_json::json!(["json"]));
    assert_eq!(caps["layouts"], serde_json::json!(["single"]));
    assert_eq!(caps["paths"], serde_json::json!(["current", "pinned"]));
    assert_eq!(caps["nodes"].as_array().unwrap().len(), 18);
}

/// Runs the framing loop over a single input line and returns the one
/// response line it wrote, parsed back into JSON.
fn response_to(line: &str) -> serde_json::Value {
    let mut output = Vec::new();
    run(line.as_bytes(), &mut output).expect("run");
    let text = String::from_utf8(output).expect("utf8 output");
    let text = text.trim();
    assert!(!text.is_empty(), "expected one response line, got none");
    serde_json::from_str(text).expect("response is JSON")
}

/// Every protocol-level failure answers the same shape on stdout: `id` is
/// the id when one parsed, else `null`, and the diagnostic is always
/// `protocol_error`/`syntax`/`/` with a message describing what was wrong.
fn assert_protocol_error(response: &serde_json::Value, expected_id: Option<u64>) {
    match expected_id {
        Some(id) => assert_eq!(response["id"], id),
        None => assert!(
            response["id"].is_null(),
            "expected id: null, got {response}"
        ),
    }
    assert_eq!(response["ok"], false);
    let diagnostic = &response["diagnostic"];
    assert_eq!(diagnostic["code"], "protocol_error");
    assert_eq!(diagnostic["stage"], "syntax");
    assert_eq!(diagnostic["cursor"], "/");
    assert!(
        diagnostic["message"]
            .as_str()
            .is_some_and(|m| !m.is_empty()),
        "expected a non-empty message, got {response}"
    );
}

#[test]
fn a_line_that_is_valid_json_but_not_an_object_is_a_protocol_error() {
    let response = response_to("[1,2]");
    assert_protocol_error(&response, None);
}

#[test]
fn a_non_integer_id_is_a_protocol_error() {
    let response = response_to(r#"{"id":"2","op":"capabilities"}"#);
    assert_protocol_error(&response, None);
}

#[test]
fn an_empty_op_is_a_protocol_error() {
    let response = response_to(r#"{"id":5,"op":""}"#);
    assert_protocol_error(&response, Some(5));
}

#[test]
fn an_id_of_zero_is_a_protocol_error() {
    let response = response_to(r#"{"id":0,"op":"capabilities"}"#);
    assert_protocol_error(&response, None);
}

#[test]
fn a_line_that_is_not_json_at_all_is_a_protocol_error() {
    let response = response_to("{not json");
    assert_protocol_error(&response, None);
}

/// A document nested past the profile's ceiling is answered over the wire, on whatever stack the
/// framing loop happens to be on — here the default test-thread stack, in the binary the process
/// main thread. The decoders recurse once per nesting level, so without `decode` stating the
/// stack it runs on, a conforming-looking document could abort the process instead of getting
/// the `nesting_too_deep` the profile promises.
#[test]
fn a_document_nested_past_the_ceiling_answers_over_the_wire() {
    // One container more than the reference reader's `MAX_DEPTH`.
    let depth = 1001;
    let document = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
    let line = serde_json::json!({
        "id": 1,
        "op": "decode",
        "version": 4,
        "profile": "json",
        "path": "current",
        "strip": true,
        "node": "Value",
        "input": document,
    });
    let response = response_to(&line.to_string());
    assert_eq!(response["id"], 1);
    assert_eq!(response["ok"], false);
    assert_eq!(response["diagnostic"]["code"], "nesting_too_deep");
    assert_eq!(response["diagnostic"]["stage"], "syntax");
}

use morphir_mck_adapter::protocol::*;

#[test]
fn a_decode_request_parses_and_unknown_fields_are_refused() {
    let (id, req) = parse_line(r#"{"id":2,"op":"decode","version":4,"profile":"json","path":"current","strip":true,"node":"Type","input":"\"a\""}"#).unwrap();
    assert_eq!(id, 2);
    assert!(matches!(req, Request::Decode(d) if d.node == NodeKind::Type && d.strip));
    let (id, diag) = parse_line(r#"{"id":3,"op":"decode","version":4,"profile":"json","path":"current","strip":true,"node":"Type","input":"","extra":1}"#).unwrap_err();
    assert_eq!(id, Some(3));
    assert_eq!(diag.code, morphir_core::ir::DiagnosticCode::InvalidJson);
}

#[test]
fn malformed_lines_report_invalid_json_without_an_id() {
    let (id, diag) = parse_line("{not json").unwrap_err();
    assert_eq!(id, None);
    assert_eq!(diag.code, morphir_core::ir::DiagnosticCode::InvalidJson);
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

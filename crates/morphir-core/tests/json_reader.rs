use morphir_core::ir::{DiagnosticCode, json};

#[test]
fn a_duplicate_member_is_refused_with_its_pointer() {
    let error = json::read(r#"{ "a": { "b": 1, "b": 2 } }"#).unwrap_err();
    assert_eq!(error.code, DiagnosticCode::DuplicateMember);
    assert_eq!(error.cursor, "/a/b");
}

#[test]
fn nesting_past_the_ceiling_is_refused() {
    let deep = format!(
        "{}1{}",
        "[".repeat(json::MAX_DEPTH + 1),
        "]".repeat(json::MAX_DEPTH + 1)
    );
    assert_eq!(
        json::read(&deep).unwrap_err().code,
        DiagnosticCode::NestingTooDeep
    );
}

#[test]
fn text_that_is_not_json_is_invalid_json() {
    assert_eq!(
        json::read("{ nope").unwrap_err().code,
        DiagnosticCode::InvalidJson
    );
}

#[test]
fn a_whole_document_round_trips_through_the_json_pair() {
    let text = r#"{ "formatVersion": 4, "distribution": { "Library": { "packageName": "example", "dependencies": {}, "def": { "modules": {} } } } }"#;
    let (file, warnings) = json::read_ir_file(text).unwrap();
    assert!(warnings.is_empty());
    assert_eq!(json::write_ir_file(&file), text);
}

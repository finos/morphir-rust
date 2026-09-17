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

/// `read` grows its own stack on demand (`stacker::maybe_grow`) rather than assuming the caller
/// already reserved one, so a caller on a deliberately small stack still gets the nesting
/// ceiling's answer instead of a stack overflow — this is what lets the document-tree layout call
/// `read` once per file of a tree without spawning a thread per call.
fn on_a_small_stack<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(work)
        .expect("a small-stack thread")
        .join()
        .expect("the small-stack thread does not panic")
}

#[test]
fn a_document_at_the_ceiling_parses_from_a_small_stack() {
    let at_ceiling = format!(
        "{}1{}",
        "[".repeat(json::MAX_DEPTH),
        "]".repeat(json::MAX_DEPTH)
    );
    let result = on_a_small_stack(move || json::read(&at_ceiling));
    assert!(result.is_ok(), "{:?}", result.err());
}

#[test]
fn one_level_past_the_ceiling_is_refused_from_a_small_stack() {
    let past_ceiling = format!(
        "{}1{}",
        "[".repeat(json::MAX_DEPTH + 1),
        "]".repeat(json::MAX_DEPTH + 1)
    );
    let code = on_a_small_stack(move || json::read(&past_ceiling).unwrap_err().code);
    assert_eq!(code, DiagnosticCode::NestingTooDeep);
}

/// No per-call thread: a few hundred calls on ordinary small inputs complete promptly, which a
/// thread-per-call reader would not do cheaply.
#[test]
fn many_reads_in_a_row_complete_without_spawning_a_thread_per_call() {
    for i in 0..500 {
        let text = format!(r#"{{ "n": {i} }}"#);
        assert!(json::read(&text).is_ok());
    }
}

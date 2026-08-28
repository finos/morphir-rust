use morphir_core::traversal::{CursorSegment, IrCursor};

#[test]
fn scoped_segment_restores_cursor() {
    let mut cursor = IrCursor::default();

    cursor.with_segment(CursorSegment::Module("rules".into()), |cursor| {
        cursor.with_segment(CursorSegment::Value("calculate".into()), |cursor| {
            assert_eq!(cursor.to_string(), "module:rules/value:calculate");
        });
        assert_eq!(cursor.to_string(), "module:rules");
    });

    assert!(cursor.is_root());
}

#[test]
fn from_segments_renders_stable_semantic_path() {
    let cursor = IrCursor::from_segments([
        CursorSegment::Package("regulation".into()),
        CursorSegment::Module("rules".into()),
        CursorSegment::Type("cash-flow".into()),
        CursorSegment::Field("amount".into()),
    ]);

    assert_eq!(
        cursor.to_string(),
        "package:regulation/module:rules/type:cash-flow/field:amount"
    );
}

#[test]
fn scoped_segment_restores_cursor_after_early_result() {
    let mut cursor = IrCursor::default();
    let result: Result<(), &str> =
        cursor.with_segment(CursorSegment::Module("rules".into()), |_| Err("stop"));

    assert_eq!(result, Err("stop"));
    assert!(cursor.is_root());
}

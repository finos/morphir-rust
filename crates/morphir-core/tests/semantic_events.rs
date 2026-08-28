use morphir_core::traversal::{CursorSegment, IrCursor, SemanticEvent, SemanticEventKind};

#[test]
fn semantic_events_carry_a_format_neutral_cursor() {
    let cursor = IrCursor::root()
        .child(CursorSegment::Module("orders".into()))
        .child(CursorSegment::Value("total".into()));
    let event = SemanticEvent::new(cursor.clone(), SemanticEventKind::End);

    assert_eq!(event.cursor(), &cursor);
    assert!(matches!(event.kind(), SemanticEventKind::End));
    assert_eq!(event.cursor().to_string(), "module:orders/value:total");
}

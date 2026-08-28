use morphir_core::migration::MigrationDiagnostic;
use morphir_core::traversal::{CursorSegment, IrCursor};

#[test]
fn migration_diagnostics_retain_the_typed_cursor() {
    let cursor = IrCursor::root().child(CursorSegment::Module("orders".into()));
    let diagnostic = MigrationDiagnostic::error(
        "unsupported-module",
        cursor.clone(),
        "module cannot be migrated",
    );

    assert_eq!(diagnostic.cursor(), &cursor);
    assert_eq!(diagnostic.path, "module:orders");
}

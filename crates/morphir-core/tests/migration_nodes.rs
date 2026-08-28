use morphir_core::migration::{
    MigrationDiagnostic, MigrationOptions, MigrationReport, Severity, V4Encoding,
};
use morphir_core::traversal::{CursorSegment, IrCursor};

#[test]
fn strict_error_has_code_and_cursor() {
    let diagnostic = MigrationDiagnostic::error(
        "unsupported-v4-expression",
        IrCursor::from_segments([CursorSegment::Value("broken".into())]),
        "Hole cannot be represented in v3",
    );

    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(diagnostic.code, "unsupported-v4-expression");
    assert_eq!(diagnostic.path, "value:broken");
}

#[test]
fn partial_policy_only_recovers_explicitly_recoverable_diagnostics() {
    let mut report = MigrationReport::new(MigrationOptions {
        allow_partial: true,
        encoding: V4Encoding::Compact,
    });
    report.push(MigrationDiagnostic::recoverable(
        "invalid-value-body",
        IrCursor::default(),
        "body replaced with an incomplete v4 body",
    ));

    assert!(report.can_publish());

    report.push(MigrationDiagnostic::error(
        "invalid-identifier",
        IrCursor::default(),
        "identifier cannot be migrated",
    ));
    assert!(!report.can_publish());
}

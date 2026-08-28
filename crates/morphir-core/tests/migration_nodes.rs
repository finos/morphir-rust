use morphir_core::ir::{classic, v4};
use morphir_core::migration::{
    MigrationContext, MigrationDiagnostic, MigrationOptions, MigrationReport, Severity, V4Encoding,
    migrate_pattern, migrate_type, migrate_value,
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

#[test]
fn migrates_every_classic_type_shape_to_a_concrete_v4_type() {
    let cases = [
        r#"["Variable",{},["a"]]"#,
        r#"["Reference",{},[[["morphir"],["s","d","k"]],[["basics"]],["int"]],[]]"#,
        r#"["Tuple",{},[["Variable",{},["a"]]]]"#,
        r#"["Record",{},[[["name"],["Variable",{},["a"]]]]]"#,
        r#"["ExtensibleRecord",{},["r"],[[["name"],["Variable",{},["a"]]]]]"#,
        r#"["Function",{},["Variable",{},["a"]],["Variable",{},["b"]]]"#,
        r#"["Unit",{}]"#,
    ];
    let mut context = MigrationContext::default();

    let migrated = cases
        .into_iter()
        .map(|json| {
            let value: classic::Type<classic::Attrs> = serde_json::from_str(json).unwrap();
            migrate_type(&value, &mut context).unwrap()
        })
        .collect::<Vec<_>>();

    assert!(matches!(migrated[0], v4::Type::Variable(_, _)));
    assert!(matches!(migrated[1], v4::Type::Reference(_, _, _)));
    assert!(matches!(migrated[2], v4::Type::Tuple(_, _)));
    assert!(matches!(migrated[3], v4::Type::Record(_, _)));
    assert!(matches!(migrated[4], v4::Type::ExtensibleRecord(_, _, _)));
    assert!(matches!(migrated[5], v4::Type::Function(_, _, _)));
    assert!(matches!(migrated[6], v4::Type::Unit(_)));
}

#[test]
fn variable_pattern_becomes_an_as_pattern_over_wildcard() {
    let classic: classic::Pattern<classic::Type<classic::Attrs>> =
        serde_json::from_str(r#"["VariablePattern",["Unit",{}],["item"]]"#).unwrap();
    let mut context = MigrationContext::default();

    let migrated = migrate_pattern(&classic, &mut context).unwrap();

    assert!(matches!(
        migrated,
        v4::Pattern::AsPattern(_, pattern, _) if matches!(*pattern, v4::Pattern::WildcardPattern(_))
    ));
}

#[test]
fn value_attributes_become_a_concrete_inferred_type() {
    let classic: classic::Value<classic::Attrs, classic::Type<classic::Attrs>> =
        serde_json::from_str(
            r#"["Literal",["Reference",{},[[["morphir"],["s","d","k"]],[["basics"]],["int"]],[]],["WholeNumberLiteral",42]]"#,
        )
        .unwrap();
    let mut context = MigrationContext::default();

    let migrated = migrate_value(&classic, &mut context).unwrap();

    assert!(matches!(
        migrated,
        v4::Value::Literal(_, v4::Literal::Integer(42))
    ));
    assert!(migrated.attributes().inferred_type.is_some());
}

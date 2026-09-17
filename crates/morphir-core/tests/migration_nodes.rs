use morphir_core::ir::{classic, v4};
use morphir_core::migration::{
    MigrationContext, MigrationDiagnostic, MigrationOptions, MigrationReport, Severity, V4Encoding,
    migrate_pattern, migrate_type, migrate_type_specification, migrate_value,
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
fn a_variable_pattern_is_not_a_classic_pattern() {
    // morphir-elm never emits a VariablePattern: a variable binding is an AsPattern over a
    // wildcard, so the classic mirror refuses the spelling rather than migrating it.
    assert!(
        serde_json::from_str::<classic::Pattern<classic::Type<classic::Attrs>>>(
            r#"["VariablePattern",["Unit",{}],["item"]]"#
        )
        .is_err()
    );
}

#[test]
fn an_as_pattern_over_a_wildcard_migrates_to_an_as_pattern() {
    let classic: classic::Pattern<classic::Type<classic::Attrs>> = serde_json::from_str(
        r#"["AsPattern",["Unit",{}],["WildcardPattern",["Unit",{}]],["item"]]"#,
    )
    .unwrap();
    let mut context = MigrationContext::default();

    let migrated = migrate_pattern(&classic, &mut context).unwrap();

    assert!(matches!(
        migrated,
        v4::Pattern::AsPattern(_, pattern, _) if matches!(*pattern, v4::Pattern::WildcardPattern(_))
    ));
}

#[test]
fn a_classic_derived_type_specification_migrates_to_the_v4_derived_specification() {
    let specification: classic::TypeSpecification<classic::Attrs> = serde_json::from_str(
        r#"["DerivedTypeSpecification",[],{"baseType":["Reference",{},[[["morphir"],["s","d","k"]],[["string"]],["string"]],[]],"fromBaseType":[[["my"],["org"]],[["module"]],["from","string"]],"toBaseType":[[["my"],["org"]],[["module"]],["to","string"]]}]"#,
    )
    .unwrap();
    let mut context = MigrationContext::default();

    let migrated = migrate_type_specification(&specification, &mut context).unwrap();

    match migrated {
        v4::TypeSpecification::DerivedTypeSpecification {
            annotations,
            type_params,
            base_type,
            from_base_type,
            to_base_type,
        } => {
            assert!(annotations.is_empty());
            assert!(type_params.is_empty());
            assert!(matches!(base_type, v4::Type::Reference(..)));
            assert_eq!(
                from_base_type.to_canonical_string(),
                "my/org:module#from-string"
            );
            assert_eq!(
                to_base_type.to_canonical_string(),
                "my/org:module#to-string"
            );
        }
        other => panic!("expected a derived specification, got {other:?}"),
    }
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
        &migrated,
        v4::Value::Literal(_, v4::Literal::Integer(n)) if n == &num_bigint::BigInt::from(42)
    ));
    assert!(migrated.attributes().inferred_type.is_some());
}

#[test]
fn a_classic_decimal_literal_migrates_keeping_its_lexeme() {
    let classic: classic::Value<classic::Attrs, classic::Attrs> =
        serde_json::from_str(r#"["Literal",{},["DecimalLiteral","10.50"]]"#).unwrap();
    let mut context = MigrationContext::default();

    let migrated = migrate_value(&classic, &mut context).unwrap();

    match migrated {
        v4::Value::Literal(_, v4::Literal::Decimal(d)) => assert_eq!(d.lexeme(), "10.50"),
        other => panic!("expected a v4 decimal literal, got {other:?}"),
    }
}

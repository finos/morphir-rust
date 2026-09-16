use morphir_core::ir::{Diagnostic, DiagnosticCode};

#[test]
fn codes_serialize_as_the_kit_spells_them() {
    let d = Diagnostic::normalization(
        DiagnosticCode::UnknownMember,
        "/Record/attrs",
        "unexpected member",
    );
    let json = serde_json::to_value(&d).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "code": "unknown_member", "stage": "normalization", "cursor": "/Record/attrs", "message": "unexpected member"
        })
    );
}

#[test]
fn every_kit_code_round_trips() {
    for code in [
        "invalid_json",
        "duplicate_member",
        "nesting_too_deep",
        "invalid_type",
        "missing_member",
        "unknown_member",
        "unknown_node",
        "ambiguous_shorthand",
        "invalid_name",
        "invalid_path",
        "invalid_fqname",
        "invalid_literal",
        "invalid_access",
        "invalid_distribution_shape",
        "legacy_spelling",
        "missing_format_version",
        "duplicate_format_version",
        "invalid_format_version_type",
        "invalid_format_version_syntax",
        "format_version_out_of_range",
        "unsupported_format_version_major",
        "unsupported_format_version_minor",
        "invalid_yaml",
        "unsupported_yaml_feature",
    ] {
        let parsed: DiagnosticCode =
            serde_json::from_value(serde_json::Value::String(code.into())).unwrap();
        assert_eq!(
            serde_json::to_value(parsed).unwrap(),
            serde_json::Value::String(code.into())
        );
    }
}

#[test]
fn a_diagnostic_survives_a_serde_error() {
    let d = Diagnostic::syntax(
        DiagnosticCode::InvalidLiteral,
        "/a/b/1",
        "octal is not decimal",
    );
    let err: serde_json::Error =
        serde::de::Error::custom(morphir_core::ir::DiagnosticError(d.clone()));
    assert_eq!(Diagnostic::from_serde_error(&err), Some(d));
    let plain: serde_json::Error = serde::de::Error::custom("something else");
    assert_eq!(Diagnostic::from_serde_error(&plain), None);
}

#[test]
fn a_document_cannot_forge_a_diagnostic_by_spelling_the_marker() {
    // A derived `Deserialize` quotes the offending member back into its message, so a document
    // that spells the marker out has its text appear in a serde error verbatim.
    let forged = Diagnostic::syntax(DiagnosticCode::InvalidLiteral, "/forged", "not mine");
    let echoed = format!(
        "unknown variant `@@morphir-diagnostic@@{}`, expected one of `Unit`, `Record`",
        serde_json::to_string(&forged).unwrap()
    );
    let error: serde_json::Error = serde::de::Error::custom(echoed);
    assert_eq!(Diagnostic::from_serde_error(&error), None);

    // Nor by appending its own text after a carried diagnostic.
    let carried = Diagnostic::syntax(DiagnosticCode::InvalidName, "/carried", "mine");
    let trailing = format!(
        "@@morphir-diagnostic@@{} and then some",
        serde_json::to_string(&carried).unwrap()
    );
    let error: serde_json::Error = serde::de::Error::custom(trailing);
    assert_eq!(Diagnostic::from_serde_error(&error), None);
}

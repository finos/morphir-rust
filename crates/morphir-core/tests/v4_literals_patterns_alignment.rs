//! Literals and patterns decode and re-encode the way the Morphir Compatibility Kit's
//! `Literal` and `Pattern` cases spell them (decisions 0005, 0006 and 0013).

use morphir_core::ir::v4::{
    Literal, Pattern, SourceLocation, SpellingMode, ValueAttributes, with_spelling_mode,
};
use morphir_core::ir::{Diagnostic, DiagnosticCode};
use serde_json::json;

fn lit(value: serde_json::Value) -> Result<Literal, Diagnostic> {
    serde_json::from_value::<Literal>(value)
        .map_err(|e| Diagnostic::from_serde_error(&e).expect("codec errors carry a Diagnostic"))
}

fn pat(value: serde_json::Value) -> Result<Pattern, Diagnostic> {
    serde_json::from_value::<Pattern>(value)
        .map_err(|e| Diagnostic::from_serde_error(&e).expect("codec errors carry a Diagnostic"))
}

fn written<T: serde::Serialize>(node: &T) -> serde_json::Value {
    serde_json::to_value(node).unwrap()
}

/// Decodes `input`, asserts it re-encodes to `canonical`, and returns the warnings recorded
/// inside the decision 0006 window.
fn normalizes(input: serde_json::Value, canonical: serde_json::Value) -> Vec<DiagnosticCode> {
    let (decoded, warnings) = with_spelling_mode(SpellingMode::Current, || pat(input.clone()));
    let decoded = decoded.unwrap_or_else(|e| panic!("{input} did not decode: {e:?}"));
    assert_eq!(written(&decoded), canonical, "re-encoding of {input}");
    warnings.into_iter().map(|warning| warning.code).collect()
}

// =============================================================================
// Literals
// =============================================================================

#[test]
fn a_literal_carries_its_payload_directly_under_its_tag() {
    for (spelling, expected) in [
        (json!({ "BoolLiteral": false }), Literal::Bool(false)),
        (json!({ "CharLiteral": "z" }), Literal::Char('z')),
        (
            json!({ "StringLiteral": "hi" }),
            Literal::String("hi".into()),
        ),
        (json!({ "IntegerLiteral": -7 }), Literal::Integer(-7)),
        (json!({ "FloatLiteral": 2.5 }), Literal::Float(2.5)),
        (
            json!({ "DecimalLiteral": "0.010" }),
            Literal::Decimal("0.010".into()),
        ),
    ] {
        assert_eq!(lit(spelling.clone()).unwrap(), expected, "{spelling}");
        assert_eq!(written(&expected), spelling);
    }
}

#[test]
fn a_float_is_written_so_it_reads_back_as_a_float() {
    // A whole-numbered float must not be written as an integer, or a reader would decode it as
    // an IntegerLiteral.
    let text = serde_json::to_string(&Literal::Float(4.0)).unwrap();
    assert!(
        text.contains("4.0") || text.contains("4e0"),
        "a float needs a decimal point or an exponent: {text}"
    );
    assert_eq!(
        serde_json::from_str::<Literal>(&text).unwrap(),
        Literal::Float(4.0)
    );
}

#[test]
fn whole_number_literal_is_an_accepted_spelling_of_integer_literal() {
    for spelling in [
        json!({ "WholeNumberLiteral": 7 }),
        json!({ "IntegerLiteral": { "value": 7 } }),
    ] {
        let (decoded, warnings) =
            with_spelling_mode(SpellingMode::Current, || lit(spelling.clone()));
        assert_eq!(decoded.unwrap(), Literal::Integer(7), "{spelling}");
        assert!(warnings.is_empty(), "{spelling} warned: {warnings:?}");
    }
    assert_eq!(
        written(&Literal::Integer(7)),
        json!({ "IntegerLiteral": 7 })
    );
}

#[test]
fn a_character_is_one_code_point_written_as_a_string() {
    let (decoded, warnings) = with_spelling_mode(SpellingMode::Current, || {
        lit(json!({ "CharLiteral": { "value": "z" } }))
    });
    assert_eq!(decoded.unwrap(), Literal::Char('z'));
    assert!(warnings.is_empty(), "{warnings:?}");

    // An astral character is one code point, so it is a single CharLiteral.
    let astral = lit(json!({ "CharLiteral": "\u{1D11E}" })).unwrap();
    assert_eq!(astral, Literal::Char('\u{1D11E}'));
    assert_eq!(written(&astral), json!({ "CharLiteral": "\u{1D11E}" }));

    for refused in [json!({ "CharLiteral": "yz" }), json!({ "CharLiteral": "" })] {
        assert_eq!(
            lit(refused.clone()).unwrap_err().code,
            DiagnosticCode::InvalidLiteral,
            "{refused}"
        );
    }
}

#[test]
fn an_integer_literal_outside_the_model_is_an_invalid_literal() {
    let beyond = json!({ "IntegerLiteral": 18446744073709551615u64 });
    assert_eq!(
        lit(beyond).unwrap_err().code,
        DiagnosticCode::InvalidLiteral
    );
    assert_eq!(
        lit(json!({ "IntegerLiteral": 1.5 })).unwrap_err().code,
        DiagnosticCode::InvalidLiteral
    );
}

#[test]
fn a_decimal_literal_keeps_its_text_rather_than_becoming_a_float() {
    let decimal = lit(json!({ "DecimalLiteral": "20.70" })).unwrap();
    assert_eq!(decimal, Literal::Decimal("20.70".into()));
    assert_eq!(
        serde_json::to_string(&decimal).unwrap(),
        r#"{"DecimalLiteral":"20.70"}"#
    );
    assert_eq!(
        lit(json!({ "DecimalLiteral": 20.70 })).unwrap_err().code,
        DiagnosticCode::InvalidLiteral
    );
}

#[test]
fn a_document_literal_keeps_its_payload_verbatim_including_large_integers() {
    // Parsed from text, because only the text carries the lexeme a host's own JSON value would
    // already have rounded.
    let source = r#"{"DocumentLiteral":{"count":9007199254740995,"rate":1.50,"tags":["b","a"]}}"#;
    let decoded: Literal = serde_json::from_str(source).unwrap();
    assert!(matches!(decoded, Literal::Document(_)));
    assert_eq!(serde_json::to_string(&decoded).unwrap(), source);
}

#[test]
fn a_document_literal_payload_is_the_document_itself() {
    // There is no `{ "value": .. }` spelling for a document: such a payload is a one-member
    // document.
    let decoded = lit(json!({ "DocumentLiteral": { "value": 1 } })).unwrap();
    assert_eq!(decoded, Literal::Document(json!({ "value": 1 })));
    assert_eq!(
        written(&decoded),
        json!({ "DocumentLiteral": { "value": 1 } })
    );
}

#[test]
fn a_tag_that_names_no_literal_is_an_unknown_node() {
    assert_eq!(
        lit(json!({ "IntLiteral": 1 })).unwrap_err().code,
        DiagnosticCode::UnknownNode
    );
}

// =============================================================================
// Patterns
// =============================================================================

#[test]
fn a_bare_literal_at_pattern_position_is_a_literal_pattern() {
    for (spelling, expected) in [
        (json!({ "IntegerLiteral": 1 }), Literal::Integer(1)),
        (json!(5), Literal::Integer(5)),
        (json!("txt"), Literal::String("txt".into())),
        (json!(true), Literal::Bool(true)),
    ] {
        let decoded = pat(spelling.clone()).unwrap();
        assert!(
            matches!(&decoded, Pattern::LiteralPattern(_, seen) if seen == &expected),
            "{spelling} decoded as {decoded:?}"
        );
    }
    assert_eq!(
        written(&pat(json!({ "IntegerLiteral": 1 })).unwrap()),
        json!({ "LiteralPattern": { "IntegerLiteral": 1 } })
    );
}

#[test]
fn a_literal_pattern_accepts_a_bare_literal_and_the_expanded_spelling() {
    let canonical = json!({ "LiteralPattern": { "IntegerLiteral": 5 } });
    for spelling in [
        canonical.clone(),
        json!({ "LiteralPattern": 5 }),
        json!({ "LiteralPattern": { "attributes": {}, "literal": { "IntegerLiteral": 5 } } }),
    ] {
        assert!(normalizes(spelling, canonical.clone()).is_empty());
    }
}

#[test]
fn a_document_cannot_be_pattern_matched() {
    assert_eq!(
        pat(json!({ "LiteralPattern": { "DocumentLiteral": { "who": "me" } } }))
            .unwrap_err()
            .code,
        DiagnosticCode::InvalidLiteral
    );
    assert_eq!(
        pat(json!({ "DocumentLiteral": { "who": "me" } }))
            .unwrap_err()
            .code,
        DiagnosticCode::InvalidLiteral
    );
}

#[test]
fn a_tuple_pattern_is_written_as_an_array_of_patterns() {
    let canonical = json!({ "TuplePattern": [{ "UnitPattern": {} }, { "WildcardPattern": {} }] });
    for spelling in [
        canonical.clone(),
        json!([{ "UnitPattern": {} }, { "WildcardPattern": {} }]),
        json!({ "TuplePattern": { "patterns": [{ "UnitPattern": {} }, { "WildcardPattern": {} }] } }),
        json!({ "TuplePattern": { "attributes": {}, "patterns": [
            { "UnitPattern": { "attributes": {} } },
            { "WildcardPattern": {} }
        ] } }),
    ] {
        assert!(normalizes(spelling, canonical.clone()).is_empty());
    }
}

#[test]
fn nullary_patterns_take_an_empty_payload_or_attributes_alone() {
    for tag in ["WildcardPattern", "EmptyListPattern", "UnitPattern"] {
        let canonical = json!({ tag: {} });
        assert!(normalizes(canonical.clone(), canonical.clone()).is_empty());
        assert!(normalizes(json!({ tag: { "attributes": {} } }), canonical).is_empty());
    }
}

#[test]
fn a_constructor_pattern_names_its_fqname_and_its_patterns() {
    let canonical = json!({ "ConstructorPattern": {
        "fqname": "morphir/SDK:result#ok",
        "patterns": [{ "WildcardPattern": {} }]
    } });
    for spelling in [
        canonical.clone(),
        json!({ "ConstructorPattern": {
            "attributes": {},
            "fqname": "morphir/SDK:result#ok",
            "patterns": [{ "WildcardPattern": {} }]
        } }),
    ] {
        assert!(normalizes(spelling, canonical.clone()).is_empty());
    }
}

#[test]
fn head_tail_and_as_patterns_name_their_parts() {
    let canonical = json!({ "HeadTailPattern": {
        "head": { "AsPattern": { "pattern": { "WildcardPattern": {} }, "name": "y" } },
        "tail": { "EmptyListPattern": {} }
    } });
    for spelling in [
        canonical.clone(),
        json!({ "HeadTailPattern": {
            "attributes": {},
            "head": { "AsPattern": { "attributes": {}, "pattern": { "WildcardPattern": {} }, "name": "y" } },
            "tail": { "EmptyListPattern": { "attributes": {} } }
        } }),
    ] {
        assert!(normalizes(spelling, canonical.clone()).is_empty());
    }
}

#[test]
fn a_pattern_keeps_its_attributes_when_they_carry_something() {
    let attributes = ValueAttributes::with_source(SourceLocation::new(7, 3, 7, 4));
    let pattern = Pattern::AsPattern(
        attributes,
        Box::new(Pattern::WildcardPattern(ValueAttributes::default())),
        morphir_core::naming::Name::from("y"),
    );
    let expected = json!({ "AsPattern": {
        "attributes": { "source": { "startLine": 7, "startColumn": 3, "endLine": 7, "endColumn": 4 } },
        "pattern": { "WildcardPattern": {} },
        "name": "y"
    } });
    assert_eq!(written(&pattern), expected);
    assert_eq!(pat(expected).unwrap(), pattern);
}

#[test]
fn the_legacy_attrs_member_decodes_with_one_warning_at_its_own_cursor() {
    let (decoded, warnings) = with_spelling_mode(SpellingMode::Current, || {
        pat(json!({ "WildcardPattern": { "attrs": {} } }))
    });
    assert!(matches!(decoded.unwrap(), Pattern::WildcardPattern(_)));
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].code, DiagnosticCode::LegacySpelling);
    assert_eq!(warnings[0].cursor, "/WildcardPattern/attrs");

    let (pinned, _) = with_spelling_mode(SpellingMode::Pinned, || {
        pat(json!({ "WildcardPattern": { "attrs": {} } }))
    });
    assert_eq!(pinned.unwrap_err().code, DiagnosticCode::UnknownMember);
}

#[test]
fn a_classic_tagged_array_is_not_a_pattern_inside_a_version_4_document() {
    assert_eq!(
        pat(json!(["WildcardPattern", {}])).unwrap_err().code,
        DiagnosticCode::UnknownNode
    );
    assert_eq!(
        pat(json!({ "VariablePattern": { "name": "y" } }))
            .unwrap_err()
            .code,
        DiagnosticCode::UnknownNode
    );
}

#[test]
fn a_misspelled_pattern_member_is_refused_at_its_own_cursor() {
    let refused =
        pat(json!({ "AsPattern": { "pattern": { "WildcardPattern": {} }, "nayme": "y" } }))
            .unwrap_err();
    assert_eq!(refused.code, DiagnosticCode::UnknownMember);
    assert_eq!(refused.cursor, "/AsPattern/nayme");

    let missing =
        pat(json!({ "AsPattern": { "pattern": { "WildcardPattern": {} } } })).unwrap_err();
    assert_eq!(missing.code, DiagnosticCode::MissingMember);
}

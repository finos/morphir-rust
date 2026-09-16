//! Value expressions decode and re-encode the way the Morphir Compatibility Kit's `Value` cases
//! spell them (decisions 0005, 0006, 0008 and 0009).

use morphir_core::ir::v4::{
    Literal, SourceLocation, SpellingMode, TypeEncoding, Value, ValueAttributes,
    with_spelling_mode, with_type_encoding,
};
use morphir_core::ir::{Diagnostic, DiagnosticCode};
use serde_json::json;

fn val(value: serde_json::Value) -> Result<Value, Diagnostic> {
    serde_json::from_value::<Value>(value)
        .map_err(|e| Diagnostic::from_serde_error(&e).expect("codec errors carry a Diagnostic"))
}

/// The canonical spelling of a decoded value: nested type expressions take their compact form.
fn canonical(node: &Value) -> serde_json::Value {
    with_type_encoding(TypeEncoding::Compact, || {
        serde_json::to_value(node).unwrap()
    })
}

/// Decodes `input`, asserts it re-encodes to `expected`, and returns the warnings recorded
/// inside the decision 0006 window.
fn normalizes(input: serde_json::Value, expected: serde_json::Value) -> Vec<DiagnosticCode> {
    let (decoded, warnings) = with_spelling_mode(SpellingMode::Current, || val(input.clone()));
    let decoded = decoded.unwrap_or_else(|e| panic!("{input} did not decode: {e:?}"));
    assert_eq!(canonical(&decoded), expected, "re-encoding of {input}");
    warnings.into_iter().map(|warning| warning.code).collect()
}

/// Asserts every spelling in `accepted` decodes to `expected` with no warning, and that
/// `expected` is what a writer produces.
fn all_normalize_to(accepted: &[serde_json::Value], expected: serde_json::Value) {
    for spelling in accepted {
        assert_eq!(
            normalizes(spelling.clone(), expected.clone()),
            Vec::<DiagnosticCode>::new(),
            "{spelling} warned"
        );
    }
}

// =============================================================================
// Shorthands: a bare scalar, string or array at a value position
// =============================================================================

#[test]
fn a_bare_array_is_a_list_and_a_bare_scalar_is_a_literal() {
    let list = json!({ "List": [
        { "Literal": { "IntegerLiteral": 1 } },
        { "Literal": { "IntegerLiteral": 2 } },
        { "Literal": { "IntegerLiteral": 3 } }
    ] });
    all_normalize_to(
        &[
            list.clone(),
            json!([1, 2, 3]),
            json!({ "List": [1, 2, 3] }),
            json!({ "List": { "items": [1, 2, 3] } }),
            json!({ "List": { "attributes": {}, "items": [1, 2, 3] } }),
        ],
        list,
    );

    assert!(matches!(
        val(json!(true)).unwrap(),
        Value::Literal(_, Literal::Bool(true))
    ));
    assert!(matches!(
        val(json!(42)).unwrap(),
        Value::Literal(_, Literal::Integer(42))
    ));
    assert_eq!(
        canonical(&val(json!(4.0)).unwrap()),
        json!({ "Literal": { "FloatLiteral": 4.0 } })
    );
}

#[test]
fn a_bare_string_is_a_variable_or_a_reference_by_its_shape() {
    assert!(matches!(val(json!("x")).unwrap(), Value::Variable(_, _)));
    assert!(matches!(
        val(json!("morphir/SDK:basics#negate")).unwrap(),
        Value::Reference(_, _)
    ));

    all_normalize_to(
        &[
            json!({ "Variable": "x" }),
            json!("x"),
            json!({ "Variable": { "attributes": {}, "name": "x" } }),
        ],
        json!({ "Variable": "x" }),
    );
    all_normalize_to(
        &[
            json!({ "Reference": "morphir/SDK:basics#add" }),
            json!("morphir/SDK:basics#add"),
            json!({ "Reference": { "attributes": {}, "fqname": "morphir/SDK:basics#add" } }),
        ],
        json!({ "Reference": "morphir/SDK:basics#add" }),
    );
}

#[test]
fn a_tuple_always_carries_its_wrapper() {
    let tuple = json!({ "Tuple": [{ "Variable": "x" }, { "Literal": { "IntegerLiteral": 1 } }] });
    all_normalize_to(
        &[
            tuple.clone(),
            json!({ "Tuple": { "elements": [{ "Variable": "x" }, { "Literal": { "IntegerLiteral": 1 } }] } }),
            json!({ "Tuple": { "attributes": {}, "elements": [{ "Variable": "x" }, 1] } }),
        ],
        tuple,
    );

    // The same array without the wrapper is a List, not a Tuple.
    assert!(matches!(
        val(json!([{ "Variable": "x" }, 1])).unwrap(),
        Value::List(_, _)
    ));
}

#[test]
fn a_literal_value_accepts_every_literal_shorthand() {
    all_normalize_to(
        &[
            json!({ "Literal": { "IntegerLiteral": 42 } }),
            json!({ "Literal": 42 }),
            json!(42),
            json!({ "Literal": { "IntegerLiteral": { "value": 42 } } }),
            json!({ "Literal": { "WholeNumberLiteral": 42 } }),
            json!({ "Literal": { "attributes": {}, "literal": { "IntegerLiteral": 42 } } }),
        ],
        json!({ "Literal": { "IntegerLiteral": 42 } }),
    );
}

// =============================================================================
// Member names decision 0006 settled
// =============================================================================

#[test]
fn if_then_else_member_names_and_the_window() {
    let canonical_form = json!({ "IfThenElse": {
        "condition": { "Literal": { "BoolLiteral": true } },
        "then": { "Literal": { "IntegerLiteral": 1 } },
        "else": { "Literal": { "IntegerLiteral": 2 } }
    } });
    all_normalize_to(
        &[
            canonical_form.clone(),
            json!({ "IfThenElse": { "attributes": {}, "condition": true, "then": 1, "else": 2 } }),
        ],
        canonical_form.clone(),
    );

    assert_eq!(
        normalizes(
            json!({ "IfThenElse": { "condition": true, "thenBranch": 1, "elseBranch": 2 } }),
            canonical_form,
        ),
        vec![
            DiagnosticCode::LegacySpelling,
            DiagnosticCode::LegacySpelling
        ]
    );
}

#[test]
fn field_access_names_its_target_and_the_window_keeps_the_older_spelling() {
    let canonical_form =
        json!({ "Field": { "target": { "Variable": "record" }, "name": "field-name" } });
    all_normalize_to(
        &[
            canonical_form.clone(),
            json!({ "Field": { "attributes": {}, "target": "record", "name": "field-name" } }),
        ],
        canonical_form.clone(),
    );

    assert_eq!(
        normalizes(
            json!({ "Field": { "subject": { "Variable": "record" }, "fieldName": "field-name" } }),
            canonical_form,
        ),
        vec![
            DiagnosticCode::LegacySpelling,
            DiagnosticCode::LegacySpelling
        ]
    );
}

#[test]
fn a_let_definition_names_its_binding_definition_and_body() {
    let definition = json!({ "ExpressionBody": {
        "inputTypes": {},
        "outputType": "morphir/SDK:basics#int",
        "body": { "Literal": { "IntegerLiteral": 1 } }
    } });
    let canonical_form = json!({ "LetDefinition": {
        "name": "x",
        "definition": definition,
        "in": { "Variable": "x" }
    } });
    all_normalize_to(
        &[
            canonical_form.clone(),
            json!({ "LetDefinition": {
                "attributes": {},
                "name": "x",
                "definition": definition,
                "in": { "Variable": "x" }
            } }),
        ],
        canonical_form.clone(),
    );

    assert_eq!(
        normalizes(
            json!({ "LetDefinition": {
                "valueName": "x",
                "valueDefinition": definition,
                "inValue": { "Variable": "x" }
            } }),
            canonical_form,
        ),
        vec![
            DiagnosticCode::LegacySpelling,
            DiagnosticCode::LegacySpelling,
            DiagnosticCode::LegacySpelling
        ]
    );
}

#[test]
fn a_let_recursion_keys_its_definitions_by_name() {
    let canonical_form = json!({ "LetRecursion": {
        "definitions": { "f": { "ExpressionBody": {
            "inputTypes": {},
            "outputType": "morphir/SDK:basics#int",
            "body": { "Variable": "f" }
        } } },
        "in": { "Variable": "f" }
    } });
    all_normalize_to(
        std::slice::from_ref(&canonical_form),
        canonical_form.clone(),
    );
}

#[test]
fn a_pattern_match_names_the_value_it_matches_on() {
    let canonical_form = json!({ "PatternMatch": {
        "value": { "Variable": "x" },
        "cases": [
            { "pattern": { "LiteralPattern": { "IntegerLiteral": 0 } },
              "body": { "Literal": { "BoolLiteral": true } } },
            { "pattern": { "WildcardPattern": {} },
              "body": { "Literal": { "BoolLiteral": false } } }
        ]
    } });
    all_normalize_to(
        std::slice::from_ref(&canonical_form),
        canonical_form.clone(),
    );

    // `subject` is a Field member name, never a PatternMatch one.
    let (refused, _) = with_spelling_mode(SpellingMode::Current, || {
        val(json!({ "PatternMatch": { "subject": "x", "cases": [] } }))
    });
    assert_eq!(
        refused.unwrap_err().code,
        DiagnosticCode::UnknownMember,
        "subject is not a PatternMatch member"
    );
}

#[test]
fn an_update_record_names_its_target_and_its_fields() {
    let canonical_form = json!({ "UpdateRecord": {
        "target": { "Variable": "record" },
        "fields": { "name": { "Literal": { "StringLiteral": "new" } } }
    } });
    all_normalize_to(
        &[
            canonical_form.clone(),
            json!({ "UpdateRecord": {
                "attributes": {},
                "target": { "Variable": "record" },
                "fields": { "name": { "Literal": { "StringLiteral": "new" } } }
            } }),
        ],
        canonical_form,
    );
}

// =============================================================================
// Records, constructors, lambdas and destructuring
// =============================================================================

#[test]
fn record_fields_live_under_fields_and_the_direct_map_warns_once() {
    let canonical_form = json!({ "Record": { "fields": {
        "name": { "Variable": "x" },
        "age": { "Literal": { "IntegerLiteral": 25 } }
    } } });
    all_normalize_to(
        &[
            canonical_form.clone(),
            json!({ "Record": { "attributes": {}, "fields": {
                "name": { "Variable": "x" },
                "age": { "Literal": { "IntegerLiteral": 25 } }
            } } }),
        ],
        canonical_form.clone(),
    );

    // `attrs` warns at the member; the direct field map warns at the wrapper.
    assert_eq!(
        normalizes(
            json!({ "Record": { "attrs": {}, "fields": {
                "name": { "Variable": "x" },
                "age": { "Literal": { "IntegerLiteral": 25 } }
            } } }),
            canonical_form.clone(),
        ),
        vec![DiagnosticCode::LegacySpelling]
    );
    let (result, warnings) = with_spelling_mode(SpellingMode::Current, || {
        val(json!({ "Record": {
            "name": { "Variable": "x" },
            "age": { "Literal": { "IntegerLiteral": 25 } }
        } }))
    });
    assert_eq!(canonical(&result.unwrap()), canonical_form);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].cursor, "/Record");
}

#[test]
fn a_constructor_and_a_field_function_carry_their_name_directly() {
    all_normalize_to(
        &[
            json!({ "Constructor": "morphir/SDK:maybe#just" }),
            json!({ "Constructor": { "attributes": {}, "fqname": "morphir/SDK:maybe#just" } }),
        ],
        json!({ "Constructor": "morphir/SDK:maybe#just" }),
    );
    all_normalize_to(
        &[
            json!({ "FieldFunction": "name" }),
            json!({ "FieldFunction": { "attributes": {}, "name": "name" } }),
        ],
        json!({ "FieldFunction": "name" }),
    );
}

#[test]
fn an_apply_a_lambda_and_a_unit_keep_their_member_names() {
    let apply = json!({ "Apply": {
        "function": { "Reference": "morphir/SDK:basics#negate" },
        "argument": { "Literal": { "IntegerLiteral": 1 } }
    } });
    all_normalize_to(
        &[
            apply.clone(),
            json!({ "Apply": {
                "attributes": {},
                "function": { "Reference": "morphir/SDK:basics#negate" },
                "argument": { "Literal": { "IntegerLiteral": 1 } }
            } }),
        ],
        apply,
    );

    let lambda = json!({ "Lambda": {
        "pattern": { "AsPattern": { "pattern": { "WildcardPattern": {} }, "name": "x" } },
        "body": { "Variable": "x" }
    } });
    all_normalize_to(
        &[
            lambda.clone(),
            json!({ "Lambda": {
                "attributes": {},
                "pattern": { "AsPattern": { "pattern": { "WildcardPattern": {} }, "name": "x" } },
                "body": { "Variable": "x" }
            } }),
        ],
        lambda,
    );

    all_normalize_to(
        &[
            json!({ "Unit": {} }),
            json!({ "Unit": { "attributes": {} } }),
        ],
        json!({ "Unit": {} }),
    );
}

#[test]
fn a_destructure_names_its_pattern_value_and_body() {
    let canonical_form = json!({ "Destructure": {
        "pattern": { "TuplePattern": [
            { "AsPattern": { "pattern": { "WildcardPattern": {} }, "name": "a" } },
            { "WildcardPattern": {} }
        ] },
        "value": { "Variable": "pair" },
        "in": { "Variable": "a" }
    } });
    all_normalize_to(
        &[
            canonical_form.clone(),
            json!({ "Destructure": {
                "attributes": {},
                "pattern": { "TuplePattern": [
                    { "AsPattern": { "pattern": { "WildcardPattern": {} }, "name": "a" } },
                    { "WildcardPattern": {} }
                ] },
                "value": { "Variable": "pair" },
                "in": { "Variable": "a" }
            } }),
        ],
        canonical_form,
    );
}

// =============================================================================
// Decision 0008: Hole stays, Native and External leave
// =============================================================================

#[test]
fn hole_keeps_its_reason_and_an_optional_expected_type() {
    let canonical_form = json!({ "Hole": {
        "reason": { "UnresolvedReference": { "target": "my-org/project:module#deleted" } }
    } });
    all_normalize_to(
        &[
            canonical_form.clone(),
            json!({ "Hole": {
                "attributes": {},
                "reason": { "UnresolvedReference": { "target": "my-org/project:module#deleted" } }
            } }),
        ],
        canonical_form,
    );

    let with_type = json!({ "Hole": {
        "reason": { "Draft": {} },
        "expectedType": "morphir/SDK:basics#int"
    } });
    all_normalize_to(std::slice::from_ref(&with_type), with_type.clone());
}

#[test]
fn native_and_external_are_not_value_expressions() {
    assert_eq!(
        val(json!({ "Native": {
            "fqname": "morphir/SDK:basics#add",
            "nativeInfo": { "hint": { "Arithmetic": {} } }
        } }))
        .unwrap_err()
        .code,
        DiagnosticCode::UnknownNode
    );
    assert_eq!(
        val(json!({ "External": {
            "externalName": "console.log",
            "targetPlatform": "javascript"
        } }))
        .unwrap_err()
        .code,
        DiagnosticCode::UnknownNode
    );
}

// =============================================================================
// Attributes
// =============================================================================

#[test]
fn a_node_that_carries_attributes_writes_the_expanded_spelling() {
    let canonical_form = json!({ "Variable": {
        "attributes": { "source": {
            "startLine": 3, "startColumn": 5, "endLine": 3, "endColumn": 6
        } },
        "name": "x"
    } });
    let decoded = val(canonical_form.clone()).unwrap();
    assert_eq!(
        decoded.attributes().source,
        Some(SourceLocation::new(3, 5, 3, 6))
    );
    assert_eq!(canonical(&decoded), canonical_form);

    // An empty attributes is never written back (decision 0005).
    assert_eq!(
        canonical(&Value::Unit(ValueAttributes::default())),
        json!({ "Unit": {} })
    );
}

#[test]
fn a_diagnostic_inside_a_value_points_at_the_spelling_the_input_used() {
    let (refused, _) = with_spelling_mode(SpellingMode::Current, || {
        val(json!({ "IfThenElse": { "condition": true, "thenBranch": null, "else": 2 } }))
    });
    let diagnostic = refused.unwrap_err();
    assert_eq!(diagnostic.cursor, "/IfThenElse/thenBranch");
}

#[test]
fn a_value_wrapper_this_reader_does_not_know_is_an_unknown_node() {
    for spelling in [
        json!({ "Draft": {} }),
        json!({ "IncompleteBody": {} }),
        json!({ "Variable": "x", "Unit": {} }),
    ] {
        assert_eq!(
            val(spelling.clone()).unwrap_err().code,
            DiagnosticCode::UnknownNode,
            "{spelling}"
        );
    }
}

//! Value expressions decode and re-encode the way the Morphir Compatibility Kit's `Value` cases
//! spell them (decisions 0005, 0006, 0008 and 0009).
//!
//! Every fixture below is this crate's own: the kit is the oracle for these rules, so no case's
//! JSON is reproduced here. The names, packages, literals and source positions are deliberately
//! unlike any the kit uses, and the rules are what the assertions pin.

use morphir_core::ir::v4::{
    Literal, SourceLocation, SpellingMode, TypeEncoding, Value, ValueAttributes, ValueBody,
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
        { "Literal": { "IntegerLiteral": 11 } },
        { "Literal": { "IntegerLiteral": 12 } },
        { "Literal": { "IntegerLiteral": 13 } }
    ] });
    all_normalize_to(
        &[
            list.clone(),
            json!([11, 12, 13]),
            json!({ "List": [11, 12, 13] }),
            json!({ "List": { "items": [11, 12, 13] } }),
            json!({ "List": { "attributes": {}, "items": [11, 12, 13] } }),
        ],
        list,
    );

    assert!(matches!(
        val(json!(false)).unwrap(),
        Value::Literal(_, Literal::Bool(false))
    ));
    assert!(matches!(
        val(json!(-8)).unwrap(),
        Value::Literal(_, Literal::Integer(n)) if n == num_bigint::BigInt::from(-8)
    ));
    // The lexeme's point is what makes a bare number a float rather than an integer.
    assert_eq!(
        canonical(&val(json!(2.5)).unwrap()),
        json!({ "Literal": { "FloatLiteral": 2.5 } })
    );
}

#[test]
fn a_bare_string_is_a_variable_or_a_reference_by_its_shape() {
    assert!(matches!(val(json!("qty")).unwrap(), Value::Variable(_, _)));
    assert!(matches!(
        val(json!("acme/shop:orders#discount")).unwrap(),
        Value::Reference(_, _)
    ));

    all_normalize_to(
        &[
            json!({ "Variable": "qty" }),
            json!("qty"),
            json!({ "Variable": { "attributes": {}, "name": "qty" } }),
        ],
        json!({ "Variable": "qty" }),
    );
    all_normalize_to(
        &[
            json!({ "Reference": "acme/shop:orders#total" }),
            json!("acme/shop:orders#total"),
            json!({ "Reference": { "attributes": {}, "fqname": "acme/shop:orders#total" } }),
        ],
        json!({ "Reference": "acme/shop:orders#total" }),
    );
}

#[test]
fn a_bare_string_that_spells_neither_a_name_nor_an_fqname_is_refused() {
    // A string at a value position is a name or an FQName; a string that is neither is a
    // malformed name, reported where it was written rather than silently becoming a variable.
    for spelling in [json!("Not A Name"), json!("Orders"), json!("")] {
        let diagnostic = val(spelling.clone()).unwrap_err();
        assert_eq!(diagnostic.code, DiagnosticCode::InvalidName, "{spelling}");
        assert_eq!(diagnostic.cursor, "", "{spelling}");
    }

    // Inside a list the cursor points at the item that carries it.
    let diagnostic = val(json!(["qty", "Not A Name"])).unwrap_err();
    assert_eq!(diagnostic.code, DiagnosticCode::InvalidName);
    assert_eq!(diagnostic.cursor, "/1");
}

#[test]
fn a_tuple_always_carries_its_wrapper() {
    let tuple =
        json!({ "Tuple": [{ "Variable": "qty" }, { "Literal": { "IntegerLiteral": 19 } }] });
    all_normalize_to(
        &[
            tuple.clone(),
            json!({ "Tuple": { "elements": [{ "Variable": "qty" }, { "Literal": { "IntegerLiteral": 19 } }] } }),
            json!({ "Tuple": { "attributes": {}, "elements": [{ "Variable": "qty" }, 19] } }),
        ],
        tuple,
    );

    // The same array without the wrapper is a List, not a Tuple.
    assert!(matches!(
        val(json!([{ "Variable": "qty" }, 19])).unwrap(),
        Value::List(_, _)
    ));
}

#[test]
fn a_literal_value_accepts_every_literal_shorthand() {
    all_normalize_to(
        &[
            json!({ "Literal": { "IntegerLiteral": 7 } }),
            json!({ "Literal": 7 }),
            json!(7),
            json!({ "Literal": { "IntegerLiteral": { "value": 7 } } }),
            json!({ "Literal": { "WholeNumberLiteral": 7 } }),
            json!({ "Literal": { "attributes": {}, "literal": { "IntegerLiteral": 7 } } }),
        ],
        json!({ "Literal": { "IntegerLiteral": 7 } }),
    );
}

// =============================================================================
// Member names decision 0006 settled
// =============================================================================

#[test]
fn if_then_else_member_names_and_the_window() {
    let canonical_form = json!({ "IfThenElse": {
        "condition": { "Literal": { "BoolLiteral": false } },
        "then": { "Literal": { "IntegerLiteral": 21 } },
        "else": { "Literal": { "IntegerLiteral": 22 } }
    } });
    all_normalize_to(
        &[
            canonical_form.clone(),
            json!({ "IfThenElse": {
                "attributes": {}, "condition": false, "then": 21, "else": 22
            } }),
        ],
        canonical_form.clone(),
    );

    assert_eq!(
        normalizes(
            json!({ "IfThenElse": { "condition": false, "thenBranch": 21, "elseBranch": 22 } }),
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
        json!({ "Field": { "target": { "Variable": "cart" }, "name": "line-total" } });
    all_normalize_to(
        &[
            canonical_form.clone(),
            json!({ "Field": { "attributes": {}, "target": "cart", "name": "line-total" } }),
        ],
        canonical_form.clone(),
    );

    assert_eq!(
        normalizes(
            json!({ "Field": { "subject": { "Variable": "cart" }, "fieldName": "line-total" } }),
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
        "outputType": "acme/shop:money#amount",
        "body": { "Literal": { "IntegerLiteral": 30 } }
    } });
    let canonical_form = json!({ "LetDefinition": {
        "name": "qty",
        "definition": definition,
        "in": { "Variable": "qty" }
    } });
    all_normalize_to(
        &[
            canonical_form.clone(),
            json!({ "LetDefinition": {
                "attributes": {},
                "name": "qty",
                "definition": definition,
                "in": { "Variable": "qty" }
            } }),
        ],
        canonical_form.clone(),
    );

    assert_eq!(
        normalizes(
            json!({ "LetDefinition": {
                "valueName": "qty",
                "valueDefinition": definition,
                "inValue": { "Variable": "qty" }
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
        "definitions": { "loop-step": { "ExpressionBody": {
            "inputTypes": {},
            "outputType": "acme/shop:money#amount",
            "body": { "Variable": "loop-step" }
        } } },
        "in": { "Variable": "loop-step" }
    } });
    all_normalize_to(
        std::slice::from_ref(&canonical_form),
        canonical_form.clone(),
    );
}

#[test]
fn a_pattern_match_names_the_value_it_matches_on() {
    let canonical_form = json!({ "PatternMatch": {
        "value": { "Variable": "qty" },
        "cases": [
            { "pattern": { "LiteralPattern": { "IntegerLiteral": 5 } },
              "body": { "Literal": { "StringLiteral": "few" } } },
            { "pattern": { "WildcardPattern": {} },
              "body": { "Literal": { "StringLiteral": "many" } } }
        ]
    } });
    all_normalize_to(
        std::slice::from_ref(&canonical_form),
        canonical_form.clone(),
    );

    // `subject` is a Field member name, never a PatternMatch one.
    let (refused, _) = with_spelling_mode(SpellingMode::Current, || {
        val(json!({ "PatternMatch": { "subject": "qty", "cases": [] } }))
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
        "target": { "Variable": "cart" },
        "fields": { "label": { "Literal": { "StringLiteral": "revised" } } }
    } });
    all_normalize_to(
        &[
            canonical_form.clone(),
            json!({ "UpdateRecord": {
                "attributes": {},
                "target": { "Variable": "cart" },
                "fields": { "label": { "Literal": { "StringLiteral": "revised" } } }
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
        "label": { "Variable": "qty" },
        "count": { "Literal": { "IntegerLiteral": 99 } }
    } } });
    all_normalize_to(
        &[
            canonical_form.clone(),
            json!({ "Record": { "attributes": {}, "fields": {
                "label": { "Variable": "qty" },
                "count": { "Literal": { "IntegerLiteral": 99 } }
            } } }),
        ],
        canonical_form.clone(),
    );

    // `attrs` warns at the member; the direct field map warns at the wrapper.
    assert_eq!(
        normalizes(
            json!({ "Record": { "attrs": {}, "fields": {
                "label": { "Variable": "qty" },
                "count": { "Literal": { "IntegerLiteral": 99 } }
            } } }),
            canonical_form.clone(),
        ),
        vec![DiagnosticCode::LegacySpelling]
    );
    let (result, warnings) = with_spelling_mode(SpellingMode::Current, || {
        val(json!({ "Record": {
            "label": { "Variable": "qty" },
            "count": { "Literal": { "IntegerLiteral": 99 } }
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
            json!({ "Constructor": "acme/shop:orders#placed" }),
            json!({ "Constructor": { "attributes": {}, "fqname": "acme/shop:orders#placed" } }),
        ],
        json!({ "Constructor": "acme/shop:orders#placed" }),
    );
    all_normalize_to(
        &[
            json!({ "FieldFunction": "line-total" }),
            json!({ "FieldFunction": { "attributes": {}, "name": "line-total" } }),
        ],
        json!({ "FieldFunction": "line-total" }),
    );
}

#[test]
fn an_apply_a_lambda_and_a_unit_keep_their_member_names() {
    let apply = json!({ "Apply": {
        "function": { "Reference": "acme/shop:orders#discount" },
        "argument": { "Literal": { "IntegerLiteral": 40 } }
    } });
    all_normalize_to(
        &[
            apply.clone(),
            json!({ "Apply": {
                "attributes": {},
                "function": { "Reference": "acme/shop:orders#discount" },
                "argument": { "Literal": { "IntegerLiteral": 40 } }
            } }),
        ],
        apply,
    );

    let lambda = json!({ "Lambda": {
        "pattern": { "AsPattern": { "pattern": { "WildcardPattern": {} }, "name": "qty" } },
        "body": { "Variable": "qty" }
    } });
    all_normalize_to(
        &[
            lambda.clone(),
            json!({ "Lambda": {
                "attributes": {},
                "pattern": { "AsPattern": { "pattern": { "WildcardPattern": {} }, "name": "qty" } },
                "body": { "Variable": "qty" }
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
            { "AsPattern": { "pattern": { "WildcardPattern": {} }, "name": "head" } },
            { "WildcardPattern": {} }
        ] },
        "value": { "Variable": "entry" },
        "in": { "Variable": "head" }
    } });
    all_normalize_to(
        &[
            canonical_form.clone(),
            json!({ "Destructure": {
                "attributes": {},
                "pattern": { "TuplePattern": [
                    { "AsPattern": { "pattern": { "WildcardPattern": {} }, "name": "head" } },
                    { "WildcardPattern": {} }
                ] },
                "value": { "Variable": "entry" },
                "in": { "Variable": "head" }
            } }),
        ],
        canonical_form,
    );
}

// =============================================================================
// Decision 0008: Hole stays a value, Native and External become definition bodies
// =============================================================================

#[test]
fn hole_keeps_its_reason_and_an_optional_expected_type() {
    let canonical_form = json!({ "Hole": {
        "reason": { "UnresolvedReference": { "target": "acme/shop:orders#retired" } }
    } });
    all_normalize_to(
        &[
            canonical_form.clone(),
            json!({ "Hole": {
                "attributes": {},
                "reason": { "UnresolvedReference": { "target": "acme/shop:orders#retired" } }
            } }),
        ],
        canonical_form,
    );

    let with_type = json!({ "Hole": {
        "reason": { "Draft": {} },
        "expectedType": "acme/shop:money#amount"
    } });
    all_normalize_to(std::slice::from_ref(&with_type), with_type.clone());
}

#[test]
fn native_and_external_are_not_value_expressions() {
    assert_eq!(
        val(json!({ "Native": {
            "fqname": "acme/shop:orders#total",
            "nativeInfo": { "hint": { "Arithmetic": {} } }
        } }))
        .unwrap_err()
        .code,
        DiagnosticCode::UnknownNode
    );
    assert_eq!(
        val(json!({ "External": {
            "externalName": "cart.total",
            "targetPlatform": "erlang"
        } }))
        .unwrap_err()
        .code,
        DiagnosticCode::UnknownNode
    );
}

#[test]
fn an_external_body_carries_one_binding_per_target_platform() {
    let body = json!({ "ExternalBody": { "externals": [
        { "targetPlatform": "erlang", "externalName": "cart:total" },
        { "targetPlatform": "elixir", "externalName": "Cart.total" }
    ] } });
    let (decoded, warnings) = with_spelling_mode(SpellingMode::Current, || {
        serde_json::from_value::<ValueBody>(body.clone())
    });
    let decoded = decoded.unwrap();
    assert!(
        matches!(&decoded, ValueBody::External { externals, fallback }
            if externals.len() == 2 && fallback.is_none())
    );
    assert!(warnings.is_empty());
    assert_eq!(serde_json::to_value(&decoded).unwrap(), body);
}

#[test]
fn the_single_binding_external_spelling_warns_at_each_member_and_closes_with_the_window() {
    let one_binding = json!({ "ExternalBody": {
        "externalName": "cart.total",
        "targetPlatform": "erlang"
    } });

    let (decoded, warnings) = with_spelling_mode(SpellingMode::Current, || {
        serde_json::from_value::<ValueBody>(one_binding.clone())
    });
    let decoded = decoded.unwrap();
    assert!(matches!(&decoded, ValueBody::External { externals, .. } if externals.len() == 1));
    assert_eq!(
        warnings
            .iter()
            .map(|warning| (warning.code, warning.cursor.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (DiagnosticCode::LegacySpelling, "/ExternalBody/externalName"),
            (
                DiagnosticCode::LegacySpelling,
                "/ExternalBody/targetPlatform"
            )
        ]
    );
    // A reader never writes the spelling back: the one binding becomes the list.
    assert_eq!(
        serde_json::to_value(&decoded).unwrap(),
        json!({ "ExternalBody": { "externals": [
            { "targetPlatform": "erlang", "externalName": "cart.total" }
        ] } })
    );

    let (refused, _) = with_spelling_mode(SpellingMode::Pinned, || {
        serde_json::from_value::<ValueBody>(one_binding)
    });
    let diagnostic = Diagnostic::from_serde_error(&refused.unwrap_err())
        .expect("codec errors carry a Diagnostic");
    assert_eq!(diagnostic.code, DiagnosticCode::UnknownMember);
    assert_eq!(diagnostic.cursor, "/ExternalBody/externalName");
}

// =============================================================================
// Attributes
// =============================================================================

#[test]
fn a_node_that_carries_attributes_writes_the_expanded_spelling() {
    let canonical_form = json!({ "Variable": {
        "attributes": { "source": {
            "startLine": 12, "startColumn": 4, "endLine": 12, "endColumn": 9
        } },
        "name": "qty"
    } });
    let decoded = val(canonical_form.clone()).unwrap();
    assert_eq!(
        decoded.attributes().source,
        Some(SourceLocation::new(12, 4, 12, 9))
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
        val(json!({ "IfThenElse": { "condition": true, "thenBranch": null, "else": 22 } }))
    });
    let diagnostic = refused.unwrap_err();
    assert_eq!(diagnostic.cursor, "/IfThenElse/thenBranch");
}

#[test]
fn a_value_wrapper_this_reader_does_not_know_is_an_unknown_node() {
    for spelling in [
        json!({ "Draft": {} }),
        json!({ "IncompleteBody": {} }),
        json!({ "Variable": "qty", "Unit": {} }),
    ] {
        assert_eq!(
            val(spelling.clone()).unwrap_err().code,
            DiagnosticCode::UnknownNode,
            "{spelling}"
        );
    }
}

//! Type expressions decode and re-encode the way the Morphir Compatibility Kit's
//! `Type` cases spell them (decisions 0004 through 0009).

use morphir_core::ir::v4::{
    SpellingMode, Type, TypeAttributes, TypeEncoding, with_spelling_mode, with_type_encoding,
};
use morphir_core::ir::{Diagnostic, DiagnosticCode, DiagnosticStage};
use morphir_core::naming::{FQName, Name};
use serde_json::json;

fn decode(v: serde_json::Value) -> Result<Type, Diagnostic> {
    serde_json::from_value::<Type>(v)
        .map_err(|e| Diagnostic::from_serde_error(&e).expect("codec errors carry a Diagnostic"))
}

fn canonical(t: &Type) -> serde_json::Value {
    with_type_encoding(TypeEncoding::Compact, || serde_json::to_value(t).unwrap())
}

#[test]
fn a_bare_array_at_type_position_is_a_tuple_even_when_it_starts_with_an_fqname() {
    let t = decode(json!([
        "morphir/SDK:basics#int",
        "morphir/SDK:string#string"
    ]))
    .unwrap();
    match &t {
        Type::Tuple(_, items) => assert_eq!(items.len(), 2),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        canonical(&t),
        json!({ "Tuple": ["morphir/SDK:basics#int", "morphir/SDK:string#string"] })
    );
}

#[test]
fn a_parameterised_reference_always_carries_its_wrapper() {
    let t = Type::Reference(
        TypeAttributes::default(),
        FQName::from_canonical_string("morphir/SDK:list#list").unwrap(),
        vec![Type::Variable(TypeAttributes::default(), Name::from("a"))],
    );
    assert_eq!(
        canonical(&t),
        json!({ "Reference": ["morphir/SDK:list#list", "a"] })
    );
    assert_eq!(
        decode(json!({ "Reference": { "fqname": "morphir/SDK:list#list", "args": ["a"] } }))
            .unwrap(),
        t
    );

    let no_arguments = Type::Reference(
        TypeAttributes::default(),
        FQName::from_canonical_string("morphir/SDK:basics#int").unwrap(),
        Vec::new(),
    );
    assert_eq!(canonical(&no_arguments), json!("morphir/SDK:basics#int"));
    assert_eq!(
        decode(json!({ "Reference": "morphir/SDK:basics#int" })).unwrap(),
        no_arguments
    );
}

#[test]
fn record_fields_live_under_fields() {
    let t =
        decode(json!({ "Record": { "fields": { "id": "morphir/SDK:string#string" } } })).unwrap();
    assert_eq!(
        canonical(&t),
        json!({ "Record": { "fields": { "id": "morphir/SDK:string#string" } } })
    );

    // The field map carried directly by the wrapper is the pre-2026-09-04 schema's spelling:
    // it decodes inside the decision 0006 window with one warning at the wrapper.
    let (legacy, warnings) = with_spelling_mode(SpellingMode::Current, || {
        decode(json!({ "Record": { "id": "morphir/SDK:string#string" } }))
    });
    assert_eq!(legacy.unwrap(), t);
    assert_eq!(
        warnings
            .iter()
            .map(|w| (w.code, w.cursor.as_str()))
            .collect::<Vec<_>>(),
        [(DiagnosticCode::LegacySpelling, "/Record")]
    );

    let (pinned, _) = with_spelling_mode(SpellingMode::Pinned, || {
        decode(json!({ "Record": { "id": "morphir/SDK:string#string" } }))
    });
    assert_eq!(pinned.unwrap_err().code, DiagnosticCode::UnknownMember);
}

#[test]
fn a_legacy_field_map_nests_and_each_level_warns_at_its_own_cursor() {
    let (decoded, warnings) = with_spelling_mode(SpellingMode::Current, || {
        decode(json!({ "Record": { "addr": { "street": "morphir/SDK:string#string" } } }))
    });
    assert_eq!(
        canonical(&decoded.unwrap()),
        json!({
            "Record": {
                "fields": {
                    "addr": { "Record": { "fields": { "street": "morphir/SDK:string#string" } } }
                }
            }
        })
    );
    assert_eq!(
        warnings
            .iter()
            .map(|w| (w.code, w.cursor.as_str()))
            .collect::<Vec<_>>(),
        [
            (DiagnosticCode::LegacySpelling, "/Record"),
            (DiagnosticCode::LegacySpelling, "/Record/addr"),
        ]
    );
}

#[test]
fn a_node_tag_inside_a_legacy_field_map_is_still_an_unknown_node() {
    // A field name is a canonical Name, which admits no mixed-case segment, so `Hole` cannot be
    // read as a field and the nested wrapper is refused where it stands.
    let (result, _) = with_spelling_mode(SpellingMode::Current, || {
        decode(json!({
            "Record": {
                "addr": {
                    "Hole": {
                        "reason": {
                            "TypeMismatch": {
                                "expected": "morphir/SDK:basics#int",
                                "found": "morphir/SDK:string#string"
                            }
                        }
                    }
                }
            }
        }))
    });
    let e = result.unwrap_err();
    assert_eq!(e.code, DiagnosticCode::UnknownNode);
    assert_eq!(e.cursor, "/Record/addr");
}

#[test]
fn an_initialism_is_a_legal_field_name_in_a_legacy_field_map() {
    let (decoded, warnings) = with_spelling_mode(SpellingMode::Current, || {
        decode(json!({ "Record": { "ID": "morphir/SDK:string#string" } }))
    });
    assert_eq!(
        canonical(&decoded.unwrap()),
        json!({ "Record": { "fields": { "ID": "morphir/SDK:string#string" } } })
    );
    assert_eq!(
        warnings
            .iter()
            .map(|w| (w.code, w.cursor.as_str()))
            .collect::<Vec<_>>(),
        [(DiagnosticCode::LegacySpelling, "/Record")]
    );
}

#[test]
fn a_scalar_where_a_type_belongs_carries_a_diagnostic() {
    for scalar in [json!(42), json!(null), json!(true), json!(1.5)] {
        let e = decode(scalar.clone()).unwrap_err();
        assert_eq!(e.code, DiagnosticCode::InvalidType, "for {scalar}");
        assert_eq!(e.stage, DiagnosticStage::Normalization, "for {scalar}");
    }
}

#[test]
fn a_diagnostic_inside_a_legacy_member_points_at_the_spelling_the_input_used() {
    let (result, _) = with_spelling_mode(SpellingMode::Current, || {
        decode(json!({ "Variable": { "attrs": 7, "name": "a" } }))
    });
    let e = result.unwrap_err();
    assert_eq!(e.code, DiagnosticCode::InvalidType);
    assert_eq!(e.cursor, "/Variable/attrs");
}

#[test]
fn a_record_wrapper_member_that_is_neither_fields_nor_a_type_is_an_unknown_member() {
    let e = decode(
        json!({ "Record": { "fields": { "id": "morphir/SDK:string#string" }, "extra": 1 } }),
    )
    .unwrap_err();
    assert_eq!(e.code, DiagnosticCode::UnknownMember);
    assert_eq!(e.cursor, "/Record/extra");
}

#[test]
fn function_member_names_follow_decision_0007_and_accept_the_window() {
    let canonical_form = json!({
        "Function": {
            "parameterType": "morphir/SDK:basics#int",
            "returnType": "morphir/SDK:string#string"
        }
    });
    let t = decode(canonical_form.clone()).unwrap();
    assert_eq!(canonical(&t), canonical_form);

    let (legacy, warnings) = with_spelling_mode(SpellingMode::Current, || {
        decode(json!({
            "Function": { "arg": "morphir/SDK:basics#int", "result": "morphir/SDK:string#string" }
        }))
    });
    assert_eq!(legacy.unwrap(), t);
    assert_eq!(
        warnings
            .iter()
            .map(|w| w.cursor.as_str())
            .collect::<Vec<_>>(),
        ["/Function/arg", "/Function/result"]
    );

    let (pinned, _) = with_spelling_mode(SpellingMode::Pinned, || {
        decode(json!({
            "Function": {
                "argumentType": "morphir/SDK:basics#int",
                "returnType": "morphir/SDK:string#string"
            }
        }))
    });
    assert_eq!(pinned.unwrap_err().code, DiagnosticCode::UnknownMember);
}

#[test]
fn attrs_is_a_legacy_spelling_of_attributes() {
    let (r, w) = with_spelling_mode(SpellingMode::Current, || {
        decode(json!({ "Variable": { "attrs": {}, "name": "a" } }))
    });
    assert_eq!(canonical(&r.unwrap()), json!("a"));
    assert_eq!(w[0].code, DiagnosticCode::LegacySpelling);
    assert_eq!(w[0].cursor, "/Variable/attrs");
}

#[test]
fn an_empty_attributes_member_is_accepted_and_never_written() {
    for accepted in [
        json!({ "Variable": { "attributes": {}, "name": "a" } }),
        json!({ "Reference": { "attributes": {}, "fqname": "morphir/SDK:basics#int" } }),
        json!({ "Unit": { "attributes": {} } }),
    ] {
        let (decoded, warnings) =
            with_spelling_mode(SpellingMode::Pinned, || decode(accepted).unwrap());
        assert!(warnings.is_empty());
        let encoded = canonical(&decoded);
        assert!(
            encoded.pointer("/Variable/attributes").is_none()
                && encoded.pointer("/Reference/attributes").is_none()
                && encoded.pointer("/Unit/attributes").is_none(),
            "an empty attributes member was written: {encoded}"
        );
    }
}

#[test]
fn a_non_empty_attributes_member_makes_the_expanded_spelling_canonical() {
    let source = json!({
        "Variable": {
            "attributes": {
                "source": { "startLine": 1, "startColumn": 1, "endLine": 1, "endColumn": 2 }
            },
            "name": "a"
        }
    });
    let t = decode(source.clone()).unwrap();
    assert_eq!(canonical(&t), source);
}

#[test]
fn a_classic_array_and_a_hole_are_unknown_nodes_at_type_position() {
    assert_eq!(
        decode(json!(["Variable", {}, ["a"]])).unwrap_err().code,
        DiagnosticCode::UnknownNode
    );
    assert_eq!(
        decode(json!({ "Hole": { "reason": { "Draft": {} } } }))
            .unwrap_err()
            .code,
        DiagnosticCode::UnknownNode
    );
    assert_eq!(
        decode(json!({ "Draft": {} })).unwrap_err().code,
        DiagnosticCode::UnknownNode
    );
}

#[test]
fn the_remaining_wrappers_keep_their_kit_member_names() {
    let tuple = decode(json!({
        "Tuple": { "elements": ["morphir/SDK:basics#int", "morphir/SDK:string#string"] }
    }))
    .unwrap();
    assert_eq!(
        canonical(&tuple),
        json!({ "Tuple": ["morphir/SDK:basics#int", "morphir/SDK:string#string"] })
    );

    let extensible = json!({
        "ExtensibleRecord": { "variable": "r", "fields": { "email": "morphir/SDK:string#string" } }
    });
    assert_eq!(canonical(&decode(extensible.clone()).unwrap()), extensible);

    let unit = json!({ "Unit": {} });
    assert_eq!(canonical(&decode(unit.clone()).unwrap()), unit);
}

#[test]
fn record_fields_keep_the_order_they_were_written_in() {
    let source = json!({
        "Record": {
            "fields": { "name": "morphir/SDK:string#string", "age": "morphir/SDK:basics#int" }
        }
    });
    let encoded = serde_json::to_string(&with_type_encoding(TypeEncoding::Compact, || {
        serde_json::to_value(decode(source).unwrap()).unwrap()
    }))
    .unwrap();
    assert_eq!(
        encoded,
        r#"{"Record":{"fields":{"name":"morphir/SDK:string#string","age":"morphir/SDK:basics#int"}}}"#
    );
}

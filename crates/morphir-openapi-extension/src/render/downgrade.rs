//! Rewrite a finished OpenAPI 3.1 document into OpenAPI 3.0.
//!
//! `render::openapi` always builds the 3.1 document first: one projection,
//! one document builder, so the two versions cannot drift. [`downgrade`]
//! walks that finished document and rewrites every 2020-12-only shape into
//! its OpenAPI 3.0 (JSON Schema Draft 4-based) equivalent:
//!
//! - `openapi` becomes `"3.0.3"`.
//! - `{"const": v}` becomes `{"enum": [v]}` — 3.0 has no `const`.
//! - `{"type": ["a", "null"]}` becomes `{"type": "a", "nullable": true}`.
//! - `{"anyOf": [X, {"type": "null"}]}` becomes `X` merged with `"nullable":
//!   true` — the only shape `Schema::Union` ever renders is a `Maybe`'s
//!   payload paired with `Schema::Null`, so that is the only `anyOf` shape
//!   collapsed; any other `anyOf`/`oneOf` is valid 3.0 as written and is
//!   left untouched.
//! - `{"prefixItems": [...], "items": false}` becomes `{"items": {"oneOf":
//!   [...]}}` — 3.0 has no `prefixItems`, so a positional tuple becomes an
//!   array whose elements can be any of the tuple's member schemas;
//!   `minItems`/`maxItems` already pin the length exactly.
//! - A `$ref` next to any other keyword — whether original or produced by
//!   one of the rewrites above — becomes `{"allOf": [{"$ref": ...}],
//!   ...siblings}`, because 3.0 tooling ignores every sibling of a `$ref`.
//! - `x-morphir-*` extension keys are valid in 3.0 and are never touched.
//! - A surviving `$defs` key is a bug, not a form to project: every named
//!   schema this document can reach already lives in `components/schemas`,
//!   so `$defs` can only mean a schema slipped in unrendered. It fails the
//!   pass with [`SchemaDiagnostic::unsupported_form`] naming the schema
//!   rather than dropping the key silently.

use serde_json::{Map, Value, json};

use crate::SchemaDiagnostic;

/// The `openapi` version string a 3.0 document declares.
const OPENAPI_VERSION: &str = "3.0.3";

/// Rewrite a finished OpenAPI 3.1 document (as [`super::openapi`] builds it)
/// into OpenAPI 3.0.
///
/// `document` is walked bottom-up: every child is rewritten before the
/// object that holds it, so a rewrite that inspects a child — the
/// null-union collapse reading its non-null member, the `prefixItems`
/// rewrite reading each tuple member — always sees that child's own final
/// 3.0 shape rather than its 3.1 original.
pub(crate) fn downgrade(document: Value) -> Result<Value, SchemaDiagnostic> {
    let rewritten = rewrite_value(document, "$")?;
    let Value::Object(mut object) = rewritten else {
        unreachable!("render::openapi always builds a JSON object document");
    };
    object.insert("openapi".to_owned(), json!(OPENAPI_VERSION));
    Ok(Value::Object(object))
}

/// Rewrite one JSON value, recursing into every object member and array
/// element before applying [`rewrite_object`] to an object node itself.
///
/// `path` is a JSON-Pointer-ish breadcrumb used only to name the schema in
/// a `$defs` bug diagnostic when no `x-morphir-fqname` sibling is present
/// to name it instead.
fn rewrite_value(value: Value, path: &str) -> Result<Value, SchemaDiagnostic> {
    match value {
        Value::Object(members) => {
            let mut rewritten = Map::new();
            for (key, child) in members {
                let child_path = format!("{path}.{key}");
                rewritten.insert(key, rewrite_value(child, &child_path)?);
            }
            rewrite_object(rewritten, path).map(Value::Object)
        }
        Value::Array(items) => {
            let rewritten = items
                .into_iter()
                .enumerate()
                .map(|(index, item)| rewrite_value(item, &format!("{path}[{index}]")))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::Array(rewritten))
        }
        other => Ok(other),
    }
}

/// Apply every object-level 3.0 rewrite to one already-child-rewritten
/// object, in the order that makes each rewrite see the others' output
/// where that matters: the null-union collapse can reintroduce a `$ref`
/// (when the `Maybe`'s payload is a reference), so the `$ref`-sibling wrap
/// runs last, after every rewrite that could add or remove a `$ref`'s
/// siblings has already run.
fn rewrite_object(
    mut members: Map<String, Value>,
    path: &str,
) -> Result<Map<String, Value>, SchemaDiagnostic> {
    if members.contains_key("$defs") {
        let name = members
            .get("x-morphir-fqname")
            .and_then(Value::as_str)
            .unwrap_or(path);
        return Err(SchemaDiagnostic::unsupported_form(
            name,
            "a '$defs' keyword survived into the OpenAPI document; every named schema must \
             already live in components/schemas, so this is a rendering bug, not a form to \
             downgrade",
        ));
    }

    rewrite_const(&mut members);
    rewrite_null_type_array(&mut members);
    rewrite_null_union(&mut members);
    rewrite_prefix_items(&mut members);
    wrap_ref_with_siblings(&mut members);

    Ok(members)
}

/// `{"const": v}` becomes `{"enum": [v]}`: 3.0's schema dialect (JSON
/// Schema Draft 4) has no `const` keyword.
fn rewrite_const(members: &mut Map<String, Value>) {
    if let Some(value) = members.remove("const") {
        members.insert("enum".to_owned(), Value::Array(vec![value]));
    }
}

/// `{"type": ["a", "null"]}` becomes `{"type": "a", "nullable": true}`.
///
/// A `type` array naming anything other than exactly one non-null type
/// alongside `"null"` never occurs from this projection — `Schema` has no
/// form that renders a `type` array at all except through this exact
/// shape — so that case is left untouched rather than guessed at.
fn rewrite_null_type_array(members: &mut Map<String, Value>) {
    let Some(Value::Array(types)) = members.get("type") else {
        return;
    };
    if !types.iter().any(|entry| entry == "null") {
        return;
    }
    let mut remaining: Vec<Value> = types
        .iter()
        .filter(|entry| *entry != "null")
        .cloned()
        .collect();
    if remaining.len() == 1 {
        members.insert("type".to_owned(), remaining.remove(0));
        members.insert("nullable".to_owned(), json!(true));
    }
}

/// `{"anyOf": [X, {"type": "null"}]}` becomes `X`'s own keys merged into
/// this object, plus `"nullable": true`.
///
/// `Schema::Union` only ever renders this exact two-member shape — a
/// `Maybe`'s payload paired with `Schema::Null` — so that is the only
/// pattern collapsed; an `anyOf` that does not match it (no `null` member,
/// or more than two members) is valid OpenAPI 3.0 as written, since 3.0's
/// Schema Object supports `anyOf`/`oneOf`/`allOf` directly, and is left
/// untouched.
fn rewrite_null_union(members: &mut Map<String, Value>) {
    let Some(Value::Array(variants)) = members.get("anyOf") else {
        return;
    };
    if variants.len() != 2 {
        return;
    }
    let Some(null_index) = variants.iter().position(is_null_schema) else {
        return;
    };
    let payload_index = 1 - null_index;
    let Value::Object(payload) = variants[payload_index].clone() else {
        return;
    };

    members.remove("anyOf");
    members.insert("nullable".to_owned(), json!(true));
    for (key, value) in payload {
        members.insert(key, value);
    }
}

/// Whether `value` is exactly `{"type": "null"}`, with no other keys.
fn is_null_schema(value: &Value) -> bool {
    matches!(value, Value::Object(object) if object.len() == 1 && object.get("type") == Some(&json!("null")))
}

/// `{"prefixItems": [...], "items": false}` becomes `{"items": {"oneOf":
/// [...]}}`: 3.0 has no `prefixItems`, so a positional tuple becomes a
/// bounded, unpositioned array. `minItems`/`maxItems` already pin the
/// length exactly and are left as they were.
fn rewrite_prefix_items(members: &mut Map<String, Value>) {
    if members.get("items") != Some(&Value::Bool(false)) {
        return;
    }
    let Some(Value::Array(_)) = members.get("prefixItems") else {
        return;
    };
    let Some(Value::Array(prefix_items)) = members.remove("prefixItems") else {
        unreachable!("just matched Some(Value::Array(_)) above");
    };
    members.insert("items".to_owned(), json!({ "oneOf": prefix_items }));
}

/// A `$ref` next to any other keyword becomes `{"allOf": [{"$ref": ...}],
/// ...siblings}`.
///
/// OpenAPI 3.0 tooling ignores every sibling of a `$ref` in a Schema
/// Object, so a sibling left in place would be silently dropped by a
/// validator rather than rejected — `allOf` is the one place a `$ref` and
/// its siblings both take effect. Runs last in [`rewrite_object`], so it
/// also catches a `$ref` [`rewrite_null_union`] just merged `nullable`
/// onto.
fn wrap_ref_with_siblings(members: &mut Map<String, Value>) {
    if members.len() <= 1 {
        return;
    }
    let Some(reference) = members.remove("$ref") else {
        return;
    };
    members.insert(
        "allOf".to_owned(),
        Value::Array(vec![json!({ "$ref": reference })]),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_the_3_0_3_version() {
        let document = json!({"openapi": "3.1.0", "info": {}});

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(downgraded["openapi"], "3.0.3");
    }

    #[test]
    fn collapses_a_null_union_over_a_scalar_into_nullable() {
        let document = json!({
            "schema": {"anyOf": [{"type": "string"}, {"type": "null"}]}
        });

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(
            downgraded["schema"],
            json!({"type": "string", "nullable": true})
        );
    }

    #[test]
    fn collapses_a_null_union_over_a_reference_into_an_allof_wrapped_ref() {
        let document = json!({
            "schema": {
                "anyOf": [
                    {"$ref": "#/components/schemas/Foo"},
                    {"type": "null"}
                ]
            }
        });

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(
            downgraded["schema"],
            json!({
                "allOf": [{"$ref": "#/components/schemas/Foo"}],
                "nullable": true
            })
        );
    }

    #[test]
    fn collapses_a_null_type_array_into_nullable() {
        let document = json!({"schema": {"type": ["string", "null"]}});

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(
            downgraded["schema"],
            json!({"type": "string", "nullable": true})
        );
    }

    #[test]
    fn wraps_a_ref_with_non_nullable_siblings_in_allof() {
        let document = json!({
            "schema": {
                "$ref": "#/components/schemas/Foo",
                "description": "A foo."
            }
        });

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(
            downgraded["schema"],
            json!({
                "allOf": [{"$ref": "#/components/schemas/Foo"}],
                "description": "A foo."
            })
        );
    }

    #[test]
    fn leaves_a_bare_ref_with_no_siblings_alone() {
        let document = json!({"schema": {"$ref": "#/components/schemas/Foo"}});

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(
            downgraded["schema"],
            json!({"$ref": "#/components/schemas/Foo"})
        );
    }

    #[test]
    fn turns_prefix_items_into_a_bounded_oneof_array() {
        let document = json!({
            "schema": {
                "type": "array",
                "prefixItems": [{"type": "integer"}, {"type": "string"}],
                "items": false,
                "minItems": 2,
                "maxItems": 2
            }
        });

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(
            downgraded["schema"],
            json!({
                "type": "array",
                "items": {"oneOf": [{"type": "integer"}, {"type": "string"}]},
                "minItems": 2,
                "maxItems": 2
            })
        );
    }

    #[test]
    fn turns_const_into_a_single_value_enum() {
        let document = json!({"schema": {"const": "circle"}});

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(downgraded["schema"], json!({"enum": ["circle"]}));
    }

    #[test]
    fn leaves_x_morphir_extension_keys_unchanged() {
        let document = json!({
            "schema": {
                "type": "string",
                "x-morphir-fqname": "acme/customer:domain#name"
            }
        });

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(
            downgraded["schema"]["x-morphir-fqname"],
            "acme/customer:domain#name"
        );
    }

    #[test]
    fn a_surviving_defs_keyword_is_an_unsupported_form_bug() {
        let document = json!({
            "components": {
                "schemas": {
                    "Foo": {
                        "type": "object",
                        "x-morphir-fqname": "acme/customer:domain#foo",
                        "$defs": {"Bar": {"type": "string"}}
                    }
                }
            }
        });

        let error = downgrade(document).expect_err("$defs must fail, not be dropped silently");

        assert_eq!(error.code(), "JSC003");
        assert_eq!(error.source(), Some("acme/customer:domain#foo"));
    }

    #[test]
    fn names_a_surviving_defs_keyword_by_path_with_no_fqname_sibling() {
        let document = json!({"$defs": {"Bar": {"type": "string"}}});

        let error = downgrade(document).expect_err("$defs must fail, not be dropped silently");

        assert_eq!(error.code(), "JSC003");
        assert_eq!(error.source(), Some("$"));
    }

    #[test]
    fn leaves_an_unrelated_anyof_untouched() {
        let document = json!({
            "schema": {
                "anyOf": [
                    {"$ref": "#/components/schemas/A"},
                    {"$ref": "#/components/schemas/B"}
                ]
            }
        });

        let downgraded = downgrade(document.clone()).expect("no unsupported forms");

        assert_eq!(downgraded["schema"], document["schema"]);
    }
}

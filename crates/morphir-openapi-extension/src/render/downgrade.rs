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
//! - `{"type": "null"}` (`Schema::Null` — Morphir's `()`, not an optional
//!   value) becomes `{"nullable": true, "enum": [null]}`: OAS 3.0.3 §4.4
//!   does not support the `null` type at all, and there is no other type
//!   for `nullable` to sit beside here, so the value is pinned with a
//!   single-member `enum` instead.
//! - `{"anyOf": [X, {"type": "null"}]}` becomes `X` merged with `"nullable":
//!   true` — the only shape `Schema::Union` ever renders is a `Maybe`'s
//!   payload paired with `Schema::Null`, so that is the only `anyOf` shape
//!   collapsed; any other `anyOf`/`oneOf` is valid 3.0 as written and is
//!   left untouched.
//! - `{"prefixItems": [...], "items": false}` becomes `{"items": {"anyOf":
//!   [...]}}` — 3.0 has no `prefixItems`, so a positional tuple becomes an
//!   array whose elements can be any of the tuple's member schemas.
//!   `anyOf`, not `oneOf`: two tuple members can share a schema (an
//!   `(Int, Int)` pair, or `(Int, Float)` where an integer satisfies both
//!   `integer` and `number`), and `oneOf` requires exactly one branch to
//!   match, which a shared or overlapping schema fails for every element —
//!   `oneOf` would make the whole array reject every instance the 3.1
//!   original accepted. `minItems`/`maxItems` already pin the length
//!   exactly.
//! - A `$ref` next to any other keyword — whether original or produced by
//!   one of the rewrites above — becomes `{"allOf": [{"$ref": ...}],
//!   ...siblings}`, because 3.0 tooling ignores every sibling of a `$ref`.
//! - `x-morphir-*` extension keys are valid in 3.0 and are never touched.
//! - A surviving `$defs` key on a Schema Object is a bug, not a form to
//!   project: every named schema this document can reach already lives in
//!   `components/schemas`, so a Schema Object carrying `$defs` can only
//!   mean a schema slipped in unrendered. It fails the pass with
//!   [`SchemaDiagnostic::unsupported_form`] naming the schema rather than
//!   dropping the key silently.
//!
//! Every one of these rules is a *keyword* rewrite: it fires on the keys of
//! a Schema Object, never on the keys of a map whose keys are arbitrary
//! names — a `properties` object (Morphir field names) or
//! `components/schemas` itself (registered schema names). A Morphir record
//! field can be named `const`; the keyword rewrite must not mistake that
//! field's *name* for the JSON Schema `const` keyword and delete it. So the
//! walk in [`rewrite_value`] threads a [`Position`] that tracks, at every
//! node, whether it is itself a Schema Object, a name-keyed map of Schema
//! Objects, or neither, and every rule in [`rewrite_object`] only ever runs
//! at [`Position::Schema`].

use serde_json::{Map, Value, json};

use crate::SchemaDiagnostic;

/// The `openapi` version string a 3.0 document declares.
const OPENAPI_VERSION: &str = "3.0.3";

/// What kind of JSON object a node in the document is, for the purpose of
/// deciding whether its own keys are schema keywords.
///
/// Threaded through [`rewrite_value`] so a keyword rewrite — `const`,
/// `$defs`, the null-union collapse, `prefixItems` — only ever inspects the
/// keys of an actual Schema Object, never the keys of a map whose keys are
/// arbitrary names (a `properties` object's Morphir field names, or
/// `components/schemas`'s registered schema names) or the fixed spec keys
/// of a structural OpenAPI object (an Operation Object, a Parameter
/// Object, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    /// This object is itself a Schema Object: every rule in
    /// [`rewrite_object`] applies to it.
    Schema,
    /// This object's own keys are arbitrary names, not schema keywords —
    /// a `properties` object or `components/schemas` itself — but every one
    /// of its *values* is a Schema Object.
    NameMap,
    /// Neither of the above: an OpenAPI structural object (the document
    /// itself, an Operation Object, a Parameter Object, a Response
    /// Object, ...) whose keys are fixed spec keys. Its values are
    /// recursed into structurally, via [`child_position`], but no keyword
    /// rule ever runs on the object itself.
    Other,
}

/// Rewrite a finished OpenAPI 3.1 document (as [`super::openapi`] builds it)
/// into OpenAPI 3.0.
///
/// `document` is walked bottom-up: every child is rewritten before the
/// object that holds it, so a rewrite that inspects a child — the
/// null-union collapse reading its non-null member, the `prefixItems`
/// rewrite reading each tuple member — always sees that child's own final
/// 3.0 shape rather than its 3.1 original. The document itself starts at
/// [`Position::Other`]: an OpenAPI document is a structural object, not a
/// schema.
pub(crate) fn downgrade(document: Value) -> Result<Value, SchemaDiagnostic> {
    let rewritten = rewrite_value(document, "$", Position::Other)?;
    let Value::Object(mut object) = rewritten else {
        unreachable!("render::openapi always builds a JSON object document");
    };
    object.insert("openapi".to_owned(), json!(OPENAPI_VERSION));
    Ok(Value::Object(object))
}

/// Rewrite one JSON value at `position`, recursing into every object member
/// and array element — at the position [`child_position`] resolves for
/// that member — before applying [`rewrite_object`] to an object node
/// itself, and only when that node's own position is [`Position::Schema`].
///
/// `path` is a JSON-Pointer-ish breadcrumb used only to name the schema in
/// a `$defs` bug diagnostic when no `x-morphir-fqname` sibling is present
/// to name it instead.
fn rewrite_value(value: Value, path: &str, position: Position) -> Result<Value, SchemaDiagnostic> {
    match value {
        Value::Object(members) => {
            let mut rewritten = Map::new();
            for (key, child) in members {
                let child_path = format!("{path}.{key}");
                let child_position = child_position(position, &key);
                rewritten.insert(key, rewrite_value(child, &child_path, child_position)?);
            }
            if position == Position::Schema {
                rewrite_object(rewritten, path).map(Value::Object)
            } else {
                Ok(Value::Object(rewritten))
            }
        }
        Value::Array(items) => {
            // Every element of an array shares the array's own position:
            // an `anyOf`/`oneOf`/`allOf`/`prefixItems` array (position
            // `Schema`, resolved by `child_position` before recursing into
            // this array) holds schemas, while a `required`/`enum` array
            // (position irrelevant, since its elements are strings) does
            // not.
            let rewritten = items
                .into_iter()
                .enumerate()
                .map(|(index, item)| rewrite_value(item, &format!("{path}[{index}]"), position))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::Array(rewritten))
        }
        other => Ok(other),
    }
}

/// The [`Position`] of the value reached by `key` inside a node at
/// `position`.
fn child_position(position: Position, key: &str) -> Position {
    match position {
        // Every value inside a name-keyed map is itself a Schema Object,
        // whatever its key: a `properties` entry is named after a Morphir
        // field, a `components/schemas` entry after a registered schema
        // name — neither name is a schema keyword.
        Position::NameMap => Position::Schema,
        Position::Schema => match key {
            "properties" => Position::NameMap,
            "additionalProperties"
            | "items"
            | "not"
            | "anyOf"
            | "oneOf"
            | "allOf"
            | "prefixItems" => Position::Schema,
            _ => Position::Other,
        },
        Position::Other => match key {
            // A Parameter Object's, MediaType Object's, or Header Object's
            // `schema` key is the one place a Schema Object appears inside
            // an otherwise-structural object.
            "schema" => Position::Schema,
            // `components/schemas` is the only name-keyed map of Schema
            // Objects reached from a structural object.
            "schemas" => Position::NameMap,
            _ => Position::Other,
        },
    }
}

/// Apply every object-level 3.0 rewrite to one already-child-rewritten
/// Schema Object, in the order that makes each rewrite see the others'
/// output where that matters: the null-union collapse can reintroduce a
/// `$ref` (when the `Maybe`'s payload is a reference), so the `$ref`-sibling
/// wrap runs last, after every rewrite that could add or remove a `$ref`'s
/// siblings has already run.
///
/// Only ever called at [`Position::Schema`] — see [`rewrite_value`] — so
/// every key this function inspects is a schema keyword, never a Morphir
/// field name or a registered schema name that happens to collide with
/// one.
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
    rewrite_null_type_scalar(&mut members);
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

/// `{"type": "null"}` becomes `{"nullable": true, "enum": [null]}`.
///
/// `Schema::Null` (Morphir's `()`, not an optional value) is the only form
/// that renders a scalar `"null"` `type`. OAS 3.0.3 §4.4 does not support
/// the `null` type, and `nullable` only ever modifies *another* type — there
/// is no other type here for it to sit beside — so the value is pinned with
/// a single-member `enum` instead, with `nullable` alongside it for tooling
/// that inspects that keyword.
///
/// This must run before a possible parent's null-union collapse inspects
/// this object: [`is_null_schema`] recognizes this rule's *output* shape,
/// not `{"type": "null"}` itself, because by the time a parent `anyOf`
/// object is rewritten, this rule has already turned its null member into
/// that output shape — every child is rewritten before its parent.
fn rewrite_null_type_scalar(members: &mut Map<String, Value>) {
    if members.get("type") == Some(&json!("null")) {
        members.remove("type");
        members.insert("nullable".to_owned(), json!(true));
        members.insert("enum".to_owned(), json!([Value::Null]));
    }
}

/// `{"anyOf": [X, {"type": "null"}]}` becomes `X`'s own keys merged into
/// this object, plus `"nullable": true`.
///
/// `Schema::Union` only ever renders this exact two-member shape — a
/// `Maybe`'s payload paired with `Schema::Null` — so that is the only
/// pattern collapsed; an `anyOf` that does not match it (no null member, or
/// more than two members) is valid OpenAPI 3.0 as written, since 3.0's
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

/// Whether `value` is the already-downgraded null placeholder
/// `{"nullable": true, "enum": [null]}` that [`rewrite_null_type_scalar`]
/// produces from `Schema::Null`'s `{"type": "null"}`.
///
/// Children are always rewritten before their parent (see
/// [`rewrite_value`]), so by the time [`rewrite_null_union`] inspects an
/// `anyOf` member, a null member has already taken this shape — this
/// checks for the post-rewrite shape, not the 3.1 original.
fn is_null_schema(value: &Value) -> bool {
    matches!(value, Value::Object(object) if object.len() == 2
        && object.get("nullable") == Some(&json!(true))
        && object.get("enum") == Some(&json!([Value::Null])))
}

/// `{"prefixItems": [...], "items": false}` becomes `{"items": {"anyOf":
/// [...]}}`: 3.0 has no `prefixItems`, so a positional tuple becomes a
/// bounded, unpositioned array. `anyOf`, not `oneOf` — see the module docs
/// for why `oneOf` would reject every instance the 3.1 original accepted
/// once two tuple members share or overlap a schema.
/// `minItems`/`maxItems` already pin the length exactly and are left as
/// they were.
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
    members.insert("items".to_owned(), json!({ "anyOf": prefix_items }));
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
///
/// (`nullable` alongside `allOf` is itself a known OAS 3.0 no-op — most
/// tooling does not apply `nullable` through an `allOf` indirection — but
/// there is no better 3.0 rendering available: a `$ref`'s only sibling-safe
/// home is `allOf`, and `nullable` has no other keyword to attach to here.
/// This is a documented limitation of the 3.0 dialect itself, not something
/// this rewrite can route around.)
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
            "paths": {"/x": {"post": {"responses": {"200": {"content": {
                "application/json": {"schema":
                    {"anyOf": [{"type": "string"}, {"type": "null"}]}
                }
            }}}}}}
        });

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(
            downgraded["paths"]["/x"]["post"]["responses"]["200"]["content"]["application/json"]["schema"],
            json!({"type": "string", "nullable": true})
        );
    }

    #[test]
    fn collapses_a_null_union_over_a_reference_into_an_allof_wrapped_ref() {
        let document = json!({
            "components": {"schemas": {"Foo": {
                "anyOf": [
                    {"$ref": "#/components/schemas/Bar"},
                    {"type": "null"}
                ]
            }}}
        });

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(
            downgraded["components"]["schemas"]["Foo"],
            json!({
                "allOf": [{"$ref": "#/components/schemas/Bar"}],
                "nullable": true
            })
        );
    }

    #[test]
    fn collapses_a_null_type_array_into_nullable() {
        let document = json!({"components": {"schemas": {"Foo": {"type": ["string", "null"]}}}});

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(
            downgraded["components"]["schemas"]["Foo"],
            json!({"type": "string", "nullable": true})
        );
    }

    #[test]
    fn turns_a_bare_null_schema_into_a_nullable_single_value_enum() {
        let document = json!({"components": {"schemas": {"Foo": {"type": "null"}}}});

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(
            downgraded["components"]["schemas"]["Foo"],
            json!({"nullable": true, "enum": [null]})
        );
    }

    #[test]
    fn wraps_a_ref_with_non_nullable_siblings_in_allof() {
        let document = json!({"components": {"schemas": {"Foo": {
            "$ref": "#/components/schemas/Bar",
            "description": "A foo."
        }}}});

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(
            downgraded["components"]["schemas"]["Foo"],
            json!({
                "allOf": [{"$ref": "#/components/schemas/Bar"}],
                "description": "A foo."
            })
        );
    }

    #[test]
    fn leaves_a_bare_ref_with_no_siblings_alone() {
        let document =
            json!({"components": {"schemas": {"Foo": {"$ref": "#/components/schemas/Bar"}}}});

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(
            downgraded["components"]["schemas"]["Foo"],
            json!({"$ref": "#/components/schemas/Bar"})
        );
    }

    #[test]
    fn turns_prefix_items_into_a_bounded_anyof_array() {
        let document = json!({"components": {"schemas": {"Foo": {
            "type": "array",
            "prefixItems": [{"type": "integer"}, {"type": "string"}],
            "items": false,
            "minItems": 2,
            "maxItems": 2
        }}}});

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(
            downgraded["components"]["schemas"]["Foo"],
            json!({
                "type": "array",
                "items": {"anyOf": [{"type": "integer"}, {"type": "string"}]},
                "minItems": 2,
                "maxItems": 2
            })
        );
    }

    /// The regression the brief's own suggestion (`oneOf`) would have
    /// caused: a tuple whose members share (or overlap) a schema, like
    /// `(Int, Int)`, has every element matching *both* branches under
    /// `oneOf` — which requires exactly one match — so the whole array
    /// would reject every instance, including the ones the 3.1 original
    /// (`prefixItems`) accepted. `anyOf` requires only at least one match,
    /// so it stays valid.
    #[test]
    fn a_same_typed_tuple_downgrade_actually_accepts_a_valid_instance() {
        let document = json!({"components": {"schemas": {"Foo": {
            "type": "array",
            "prefixItems": [{"type": "integer"}, {"type": "integer"}],
            "items": false,
            "minItems": 2,
            "maxItems": 2
        }}}});

        let downgraded = downgrade(document).expect("no unsupported forms");
        let schema = &downgraded["components"]["schemas"]["Foo"];

        let validator = jsonschema::validator_for(schema)
            .expect("a valid JSON Schema, ignoring the OAS-only 'nullable' keyword");
        assert!(
            validator.is_valid(&json!([1, 2])),
            "a same-typed tuple's downgraded schema must accept a matching instance: {schema}"
        );
        assert!(
            !validator.is_valid(&json!([1, 2, 3])),
            "the array is still bounded to exactly two items: {schema}"
        );
        assert!(
            !validator.is_valid(&json!(["a", "b"])),
            "elements are still bounded to integers: {schema}"
        );
    }

    #[test]
    fn turns_const_into_a_single_value_enum() {
        let document = json!({"components": {"schemas": {"Foo": {"const": "circle"}}}});

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(
            downgraded["components"]["schemas"]["Foo"],
            json!({"enum": ["circle"]})
        );
    }

    #[test]
    fn leaves_x_morphir_extension_keys_unchanged() {
        let document = json!({"components": {"schemas": {"Foo": {
            "type": "string",
            "x-morphir-fqname": "acme/customer:domain#name"
        }}}});

        let downgraded = downgrade(document).expect("no unsupported forms");

        assert_eq!(
            downgraded["components"]["schemas"]["Foo"]["x-morphir-fqname"],
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
        let document = json!({
            "components": {"schemas": {"Foo": {"type": "object", "$defs": {"Bar": {"type": "string"}}}}}
        });

        let error = downgrade(document).expect_err("$defs must fail, not be dropped silently");

        assert_eq!(error.code(), "JSC003");
        assert_eq!(error.source(), Some("$.components.schemas.Foo"));
    }

    #[test]
    fn leaves_an_unrelated_anyof_untouched() {
        let document = json!({"components": {"schemas": {"Foo": {
            "anyOf": [
                {"$ref": "#/components/schemas/A"},
                {"$ref": "#/components/schemas/B"}
            ]
        }}}});

        let downgraded = downgrade(document.clone()).expect("no unsupported forms");

        assert_eq!(
            downgraded["components"]["schemas"]["Foo"],
            document["components"]["schemas"]["Foo"]
        );
    }

    /// Reproduces the exact bug an important review finding flagged: a
    /// Morphir record field can be named `const` (a perfectly ordinary
    /// Morphir identifier), which renders as the property key `"const"`
    /// inside a `properties` object. A keyword-blind rewrite would read
    /// that key as the `const` keyword and delete the field, replacing it
    /// with a bogus `enum` built from the field's own schema. `Position`
    /// scoping must leave it alone: `properties`' own keys are field
    /// names, not schema keywords, so no rule in `rewrite_object` ever
    /// runs on the `properties` object itself.
    #[test]
    fn a_record_field_literally_named_const_is_not_treated_as_a_keyword() {
        let document = json!({"components": {"schemas": {"Thing": {
            "type": "object",
            "properties": {
                "const": {"type": "string"},
                "name": {"type": "string"}
            },
            "required": ["const", "name"]
        }}}});

        let downgraded = downgrade(document).expect("no unsupported forms");

        let properties = &downgraded["components"]["schemas"]["Thing"]["properties"];
        assert_eq!(
            properties["const"],
            json!({"type": "string"}),
            "a field named 'const' must survive untouched, not become an enum: {properties}"
        );
        assert_eq!(properties["name"], json!({"type": "string"}));
    }

    /// The same bug class the review flagged for `const`, for `$defs`: a
    /// field literally named `$defs` must not trip the `$defs`-survived
    /// bug diagnostic, because `properties`' own keys are never inspected
    /// as schema keywords.
    #[test]
    fn a_record_field_literally_named_defs_does_not_trip_the_bug_diagnostic() {
        let document = json!({"components": {"schemas": {"Thing": {
            "type": "object",
            "properties": {"$defs": {"type": "string"}}
        }}}});

        let downgraded = downgrade(document)
            .expect("a field literally named '$defs' is user data, not a survived keyword");

        assert_eq!(
            downgraded["components"]["schemas"]["Thing"]["properties"]["$defs"],
            json!({"type": "string"})
        );
    }
}

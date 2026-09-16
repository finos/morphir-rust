//! Serde implementations for Morphir IR v4 nodes.
//!
//! A type expression has a compact spelling and an expanded one whose payload may open with
//! `attributes`:
//! - `"a"` is a type variable and `"morphir/SDK:basics#int"` a reference with no arguments
//! - a bare array is a Tuple, so a parameterized reference always carries its wrapper:
//!   `{ "Reference": ["morphir/SDK:list#list", "a"] }`
//! - `{ "Record": { "fields": { "name": "morphir/SDK:string#string" } } }`
//! - `{ "Variable": { "attributes": { ... }, "name": "a" } }`
//!
//! Classic tagged arrays are decoded by `ir::classic`, never here: inside a version 4
//! document `["Variable", {}, ["a"]]` is an unknown node.

use indexmap::IndexMap;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::fmt;

use super::attributes::{TypeAttributes, ValueAttributes};
use super::legacy::{accept_member, record_legacy_form_warning};
use super::literal::Literal;
use super::pattern::Pattern;
use super::serde_v4;
use super::type_def::ConstructorArg;
use super::types::{Field, Type};
use super::value::{
    HoleReason, InputType, LetBinding, NativeInfo, PatternCase, RecordFieldEntry, Value,
    ValueDefinition,
};
use crate::ir::{Diagnostic, DiagnosticCode, DiagnosticError};
use crate::naming::{FQName, Name};

fn parse_canonical_name<E: de::Error>(source: &str) -> Result<Name, E> {
    Name::from_canonical_string(source).map_err(E::custom)
}

// =============================================================================
// Type Serialization
// =============================================================================

impl Serialize for Type {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Delegate to V4 object wrapper format
        serde_v4::serialize_type(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Type {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(TypeVisitor {
            cursor: String::new(),
        })
    }
}

/// The v4 type wrapper tags.
///
/// A bare array is a Tuple, so a leading element that names one of these is not a tuple item
/// but a Classic tagged array, which a v4 reader refuses as an unknown node.
const TYPE_TAGS: &[&str] = &[
    "Variable",
    "Reference",
    "Tuple",
    "Record",
    "ExtensibleRecord",
    "Function",
    "Unit",
];

/// Decodes a type expression, carrying the JSON pointer of the node being read so every
/// diagnostic and every `legacy_spelling` warning is located.
struct TypeVisitor {
    cursor: String,
}

fn carry<E: de::Error>(diagnostic: Diagnostic) -> E {
    E::custom(DiagnosticError(diagnostic))
}

impl<'de> Visitor<'de> for TypeVisitor {
    type Value = Type;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(
            "a type expression: the string \"a\" or \"morphir/SDK:basics#int\", \
             an array of type expressions, \
             or a wrapper such as { \"Record\": { \"fields\": {} } }",
        )
    }

    fn visit_str<E>(self, v: &str) -> Result<Type, E>
    where
        E: de::Error,
    {
        decode_type(&JsonValue::String(v.to_owned()), &self.cursor).map_err(carry)
    }

    fn visit_map<M>(self, map: M) -> Result<Type, M::Error>
    where
        M: MapAccess<'de>,
    {
        let value = JsonValue::deserialize(de::value::MapAccessDeserializer::new(map))?;
        decode_type(&value, &self.cursor).map_err(carry)
    }

    fn visit_seq<V>(self, seq: V) -> Result<Type, V::Error>
    where
        V: SeqAccess<'de>,
    {
        let value = JsonValue::deserialize(de::value::SeqAccessDeserializer::new(seq))?;
        decode_type(&value, &self.cursor).map_err(carry)
    }

    // A scalar is refused here rather than through the default visitor methods, which build a
    // plain serde error that carries no code or cursor. `deserialize_any` dispatches every
    // narrower integer and float width to these, so the four below cover every JSON scalar.

    fn visit_bool<E>(self, _value: bool) -> Result<Type, E>
    where
        E: de::Error,
    {
        Err(self.refuse_scalar())
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Type, E>
    where
        E: de::Error,
    {
        Err(self.refuse_scalar())
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Type, E>
    where
        E: de::Error,
    {
        Err(self.refuse_scalar())
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Type, E>
    where
        E: de::Error,
    {
        Err(self.refuse_scalar())
    }

    fn visit_unit<E>(self) -> Result<Type, E>
    where
        E: de::Error,
    {
        Err(self.refuse_scalar())
    }

    fn visit_none<E>(self) -> Result<Type, E>
    where
        E: de::Error,
    {
        Err(self.refuse_scalar())
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Type, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

impl TypeVisitor {
    fn refuse_scalar<E: de::Error>(&self) -> E {
        carry(invalid_type(&self.cursor, "expected a type expression"))
    }
}

fn invalid_type(cursor: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::normalization(DiagnosticCode::InvalidType, cursor, message)
}

fn unknown_node(cursor: &str, seen: &str) -> Diagnostic {
    Diagnostic::normalization(
        DiagnosticCode::UnknownNode,
        cursor,
        format!("{seen} is not a type expression"),
    )
}

/// A canonical string that carries both an `:` and a `#` spells an FQName; anything else at a
/// type position spells a type variable's name.
fn looks_like_fqname(text: &str) -> bool {
    text.contains(':') && text.contains('#')
}

/// Whether a JSON value is shaped like a type expression, without decoding it.
///
/// Used to tell the Record wrapper carrying its field map directly from a Record wrapper
/// carrying a misspelled member. A structural test keeps the check free of side effects: a
/// trial decode would record the nested node's `legacy_spelling` warnings twice.
fn looks_like_type(value: &JsonValue) -> bool {
    match value {
        JsonValue::String(_) | JsonValue::Array(_) => true,
        JsonValue::Object(members) => {
            members.len() == 1
                && members
                    .keys()
                    .next()
                    .is_some_and(|tag| TYPE_TAGS.contains(&tag.as_str()))
        }
        _ => false,
    }
}

/// Whether an object is a Record's field map carried directly, the spelling the schema
/// documented until 2026-09-04.
///
/// A field name is a canonical `Name`, which is never capitalised the way a wrapper tag is, so
/// a member naming a node means the object is a wrapper rather than a field map. That check is
/// what keeps `{ "Hole": { "reason": { ... } } }` an unknown node instead of a record with a
/// field called `hole`.
fn is_legacy_field_map(members: &serde_json::Map<String, JsonValue>) -> bool {
    !members.is_empty()
        && !members.contains_key("fields")
        && !members.contains_key("attributes")
        && !members.contains_key("attrs")
        && !members
            .keys()
            .any(|member| TYPE_TAGS.contains(&member.as_str()))
        && members.values().all(looks_like_field_type)
}

/// A field's value inside a legacy field map is a type expression, or another legacy field map:
/// the spelling nests, and each level earns its own warning.
fn looks_like_field_type(value: &JsonValue) -> bool {
    match value {
        JsonValue::Object(members) => looks_like_type(value) || is_legacy_field_map(members),
        other => looks_like_type(other),
    }
}

/// Reads a wrapper payload's members, mapping each spelling onto the node's canonical member
/// name through the decision 0006 window table.
fn wrapper_members<'a>(
    node: &str,
    payload: &'a JsonValue,
    cursor: &str,
    canonical_members: &[&'static str],
) -> Result<Members<'a>, Diagnostic> {
    let payload = payload
        .as_object()
        .ok_or_else(|| invalid_type(cursor, format!("the {node} payload must be an object")))?;

    let mut accepted = IndexMap::new();
    for (seen, member) in payload {
        let member_cursor = format!("{cursor}/{seen}");
        let canonical = match canonical_members.iter().find(|name| *name == seen) {
            Some(name) => *name,
            None => {
                let mapped = accept_member(node, seen, &member_cursor)?;
                *canonical_members
                    .iter()
                    .find(|name| **name == mapped)
                    .ok_or_else(|| {
                        Diagnostic::normalization(
                            DiagnosticCode::UnknownMember,
                            &member_cursor,
                            format!("unexpected member {seen}"),
                        )
                    })?
            }
        };
        let member = Member {
            value: member,
            seen,
        };
        if accepted.insert(canonical, member).is_some() {
            return Err(Diagnostic::normalization(
                DiagnosticCode::DuplicateMember,
                &member_cursor,
                format!("duplicate member {canonical}"),
            ));
        }
    }
    Ok(accepted)
}

/// One accepted member: its value, and the spelling the input actually used, so a diagnostic
/// inside a member written the legacy way points at the member the author can find.
struct Member<'a> {
    value: &'a JsonValue,
    seen: &'a str,
}

type Members<'a> = IndexMap<&'static str, Member<'a>>;

/// The JSON pointer of `name`, spelled the way the input spelled it.
fn member_cursor(members: &Members<'_>, name: &str, cursor: &str) -> String {
    match members.get(name) {
        Some(member) => format!("{cursor}/{}", member.seen),
        None => format!("{cursor}/{name}"),
    }
}

fn decode_attributes(members: &Members<'_>, cursor: &str) -> Result<TypeAttributes, Diagnostic> {
    match members.get("attributes") {
        None => Ok(TypeAttributes::default()),
        Some(member) => serde_json::from_value(member.value.clone())
            .map_err(|error| invalid_type(&format!("{cursor}/{}", member.seen), error.to_string())),
    }
}

fn required<'a>(
    members: &Members<'a>,
    name: &str,
    cursor: &str,
) -> Result<&'a JsonValue, Diagnostic> {
    members.get(name).map(|member| member.value).ok_or_else(|| {
        Diagnostic::normalization(
            DiagnosticCode::MissingMember,
            cursor,
            format!("missing member {name}"),
        )
    })
}

fn decode_name(value: &JsonValue, cursor: &str) -> Result<Name, Diagnostic> {
    let text = value
        .as_str()
        .ok_or_else(|| invalid_type(cursor, "a name must be a canonical string"))?;
    Name::from_canonical_string(text)
        .map_err(|error| Diagnostic::normalization(DiagnosticCode::InvalidName, cursor, error))
}

fn decode_fqname(value: &JsonValue, cursor: &str) -> Result<FQName, Diagnostic> {
    let text = value
        .as_str()
        .ok_or_else(|| invalid_type(cursor, "an FQName must be a canonical string"))?;
    FQName::from_canonical_string(text)
        .map_err(|error| Diagnostic::normalization(DiagnosticCode::InvalidFqname, cursor, error))
}

fn decode_type_list(value: &JsonValue, cursor: &str) -> Result<Vec<Type>, Diagnostic> {
    let items = value
        .as_array()
        .ok_or_else(|| invalid_type(cursor, "expected an array of type expressions"))?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| decode_type(item, &format!("{cursor}/{index}")))
        .collect()
}

/// Decodes a field map: an object keyed by field name, whose order is the field order.
///
/// `decode_field_type` reads each field's type, which is what separates the canonical `fields`
/// member from a legacy field map: only the latter lets a field's own value be another legacy
/// field map.
fn decode_fields(
    value: &JsonValue,
    cursor: &str,
    decode_field_type: fn(&JsonValue, &str) -> Result<Type, Diagnostic>,
) -> Result<Vec<Field>, Diagnostic> {
    let fields = value
        .as_object()
        .ok_or_else(|| invalid_type(cursor, "fields must be an object keyed by field name"))?;
    fields
        .iter()
        .map(|(name, tpe)| {
            let field_cursor = format!("{cursor}/{name}");
            Ok(Field {
                name: Name::from_canonical_string(name).map_err(|error| {
                    Diagnostic::normalization(DiagnosticCode::InvalidName, &field_cursor, error)
                })?,
                tpe: decode_field_type(tpe, &field_cursor)?,
            })
        })
        .collect()
}

/// Decodes a field of a legacy field map, which may itself be another legacy field map.
///
/// Only reachable from inside a legacy Record, so a bare object at an ordinary type position —
/// `{ "Hole": ... }`, say — is still an unknown node.
fn decode_type_or_legacy_field_map(value: &JsonValue, cursor: &str) -> Result<Type, Diagnostic> {
    if let JsonValue::Object(members) = value
        && is_legacy_field_map(members)
    {
        return decode_legacy_field_map(value, members, cursor);
    }
    decode_type(value, cursor)
}

/// Decodes a Record written as its field map directly, warning at the object that carries it.
fn decode_legacy_field_map(
    value: &JsonValue,
    members: &serde_json::Map<String, JsonValue>,
    cursor: &str,
) -> Result<Type, Diagnostic> {
    let first = members
        .keys()
        .next()
        .expect("a legacy field map is never empty");
    record_legacy_form_warning(cursor, first)?;
    Ok(Type::Record(
        TypeAttributes::default(),
        decode_fields(value, cursor, decode_type_or_legacy_field_map)?,
    ))
}

/// Decodes one type expression at `cursor`.
///
/// The spellings are the ones the Morphir Compatibility Kit's `Type` cases pin: a bare string
/// is a variable or a no-argument reference, a bare array is a Tuple, and every other node is
/// a single-member wrapper whose payload may open with `attributes`.
fn decode_type(value: &JsonValue, cursor: &str) -> Result<Type, Diagnostic> {
    match value {
        JsonValue::String(text) => {
            if looks_like_fqname(text) {
                Ok(Type::Reference(
                    TypeAttributes::default(),
                    decode_fqname(value, cursor)?,
                    Vec::new(),
                ))
            } else {
                Ok(Type::Variable(
                    TypeAttributes::default(),
                    decode_name(value, cursor)?,
                ))
            }
        }
        JsonValue::Array(items) => {
            if let Some(JsonValue::String(head)) = items.first() {
                let names_a_node = TYPE_TAGS.contains(&head.as_str());
                let spells_nothing = Name::from_canonical_string(head).is_err()
                    && FQName::from_canonical_string(head).is_err();
                if names_a_node || spells_nothing {
                    return Err(unknown_node(cursor, head));
                }
            }
            Ok(Type::Tuple(
                TypeAttributes::default(),
                decode_type_list(value, cursor)?,
            ))
        }
        JsonValue::Object(wrapper) => {
            let mut entries = wrapper.iter();
            let (tag, payload) = entries
                .next()
                .ok_or_else(|| unknown_node(cursor, "an empty object"))?;
            if entries.next().is_some() {
                return Err(unknown_node(cursor, "an object with more than one member"));
            }
            decode_wrapper(tag, payload, cursor)
        }
        _ => Err(invalid_type(
            cursor,
            "a type expression is a string, an array or a wrapper object",
        )),
    }
}

fn decode_wrapper(tag: &str, payload: &JsonValue, cursor: &str) -> Result<Type, Diagnostic> {
    let at = format!("{cursor}/{tag}");
    match tag {
        "Variable" => {
            let members = wrapper_members(tag, payload, &at, &["attributes", "name"])?;
            let attributes = decode_attributes(&members, &at)?;
            let name = decode_name(
                required(&members, "name", &at)?,
                &member_cursor(&members, "name", &at),
            )?;
            Ok(Type::Variable(attributes, name))
        }
        "Reference" => {
            if payload.is_string() {
                return Ok(Type::Reference(
                    TypeAttributes::default(),
                    decode_fqname(payload, &at)?,
                    Vec::new(),
                ));
            }
            if let JsonValue::Array(items) = payload {
                let head = items.first().ok_or_else(|| {
                    Diagnostic::normalization(
                        DiagnosticCode::MissingMember,
                        &at,
                        "a Reference array begins with an FQName",
                    )
                })?;
                let fqname = decode_fqname(head, &format!("{at}/0"))?;
                let args = items[1..]
                    .iter()
                    .enumerate()
                    .map(|(index, item)| decode_type(item, &format!("{at}/{}", index + 1)))
                    .collect::<Result<_, _>>()?;
                return Ok(Type::Reference(TypeAttributes::default(), fqname, args));
            }
            let members = wrapper_members(tag, payload, &at, &["attributes", "fqname", "args"])?;
            let attributes = decode_attributes(&members, &at)?;
            let fqname = decode_fqname(
                required(&members, "fqname", &at)?,
                &member_cursor(&members, "fqname", &at),
            )?;
            let args = match members.get("args") {
                None => Vec::new(),
                Some(member) => decode_type_list(member.value, &format!("{at}/{}", member.seen))?,
            };
            Ok(Type::Reference(attributes, fqname, args))
        }
        "Tuple" => {
            if payload.is_array() {
                return Ok(Type::Tuple(
                    TypeAttributes::default(),
                    decode_type_list(payload, &at)?,
                ));
            }
            let members = wrapper_members(tag, payload, &at, &["attributes", "elements"])?;
            let attributes = decode_attributes(&members, &at)?;
            let elements = decode_type_list(
                required(&members, "elements", &at)?,
                &member_cursor(&members, "elements", &at),
            )?;
            Ok(Type::Tuple(attributes, elements))
        }
        "Record" => {
            if let JsonValue::Object(payload_members) = payload
                && is_legacy_field_map(payload_members)
            {
                return decode_legacy_field_map(payload, payload_members, &at);
            }
            let members = wrapper_members(tag, payload, &at, &["attributes", "fields"])?;
            let attributes = decode_attributes(&members, &at)?;
            let fields = decode_fields(
                required(&members, "fields", &at)?,
                &member_cursor(&members, "fields", &at),
                decode_type,
            )?;
            Ok(Type::Record(attributes, fields))
        }
        "ExtensibleRecord" => {
            let members =
                wrapper_members(tag, payload, &at, &["attributes", "variable", "fields"])?;
            let attributes = decode_attributes(&members, &at)?;
            let variable = decode_name(
                required(&members, "variable", &at)?,
                &member_cursor(&members, "variable", &at),
            )?;
            let fields = decode_fields(
                required(&members, "fields", &at)?,
                &member_cursor(&members, "fields", &at),
                decode_type,
            )?;
            Ok(Type::ExtensibleRecord(attributes, variable, fields))
        }
        "Function" => {
            let members = wrapper_members(
                tag,
                payload,
                &at,
                &["attributes", "parameterType", "returnType"],
            )?;
            let attributes = decode_attributes(&members, &at)?;
            let parameter = decode_type(
                required(&members, "parameterType", &at)?,
                &member_cursor(&members, "parameterType", &at),
            )?;
            let result = decode_type(
                required(&members, "returnType", &at)?,
                &member_cursor(&members, "returnType", &at),
            )?;
            Ok(Type::Function(
                attributes,
                Box::new(parameter),
                Box::new(result),
            ))
        }
        "Unit" => {
            let members = wrapper_members(tag, payload, &at, &["attributes"])?;
            Ok(Type::Unit(decode_attributes(&members, &at)?))
        }
        // Decision 0008 makes incompleteness a property of a definition, so `Hole` and `Draft`
        // where a type expression belongs are unknown nodes.
        _ => Err(unknown_node(cursor, tag)),
    }
}

// =============================================================================
// Field Serialization
// =============================================================================

impl Serialize for Field {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Field", 2)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("tpe", &self.tpe)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for Field {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum FieldName {
            Name,
            Tpe,
        }

        struct FieldVisitor;

        impl<'de> Visitor<'de> for FieldVisitor {
            type Value = Field;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a field object with name and tpe")
            }

            fn visit_map<V>(self, mut map: V) -> Result<Field, V::Error>
            where
                V: de::MapAccess<'de>,
            {
                let mut name = None;
                let mut tpe = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        FieldName::Name => {
                            if name.is_some() {
                                return Err(de::Error::duplicate_field("name"));
                            }
                            name = Some(map.next_value()?);
                        }
                        FieldName::Tpe => {
                            if tpe.is_some() {
                                return Err(de::Error::duplicate_field("tpe"));
                            }
                            tpe = Some(map.next_value()?);
                        }
                    }
                }

                let name = name.ok_or_else(|| de::Error::missing_field("name"))?;
                let tpe = tpe.ok_or_else(|| de::Error::missing_field("tpe"))?;
                Ok(Field { name, tpe })
            }
        }

        deserializer.deserialize_struct("Field", &["name", "tpe"], FieldVisitor)
    }
}

// =============================================================================
// Pattern Serialization
// =============================================================================

impl Serialize for Pattern {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Delegate to V4 object wrapper format
        serde_v4::serialize_pattern(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Pattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Use deserialize_any to accept V4 objects and Classic arrays
        deserializer.deserialize_any(PatternVisitor)
    }
}

struct PatternVisitor;

impl<'de> Visitor<'de> for PatternVisitor {
    type Value = Pattern;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(
            "V4 object { \"WildcardPattern\": {} } or Classic array [\"WildcardPattern\", attrs]",
        )
    }

    /// V4 object wrapper format: { "WildcardPattern": {} }
    fn visit_map<M>(self, mut map: M) -> Result<Pattern, M::Error>
    where
        M: MapAccess<'de>,
    {
        let (tag, value): (String, serde_json::Value) = map
            .next_entry()?
            .ok_or_else(|| de::Error::custom("expected object wrapper with single key"))?;

        match tag.as_str() {
            "WildcardPattern" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Pattern::WildcardPattern(attrs))
            }
            "AsPattern" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    pattern: Pattern,
                    name: String,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let name = parse_canonical_name::<M::Error>(&content.name)?;
                Ok(Pattern::AsPattern(attrs, Box::new(content.pattern), name))
            }
            "TuplePattern" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    elements: Vec<Pattern>,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Pattern::TuplePattern(attrs, content.elements))
            }
            "ConstructorPattern" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    fqname: String,
                    args: Vec<Pattern>,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let fqname = FQName::from_canonical_string(&content.fqname)
                    .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
                Ok(Pattern::ConstructorPattern(attrs, fqname, content.args))
            }
            "EmptyListPattern" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Pattern::EmptyListPattern(attrs))
            }
            "HeadTailPattern" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    head: Pattern,
                    tail: Pattern,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Pattern::HeadTailPattern(
                    attrs,
                    Box::new(content.head),
                    Box::new(content.tail),
                ))
            }
            "LiteralPattern" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    literal: Literal,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Pattern::LiteralPattern(attrs, content.literal))
            }
            "UnitPattern" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Pattern::UnitPattern(attrs))
            }
            _ => Err(de::Error::unknown_variant(
                &tag,
                &[
                    "WildcardPattern",
                    "AsPattern",
                    "TuplePattern",
                    "ConstructorPattern",
                    "EmptyListPattern",
                    "HeadTailPattern",
                    "LiteralPattern",
                    "UnitPattern",
                ],
            )),
        }
    }

    /// Classic tagged array format: ["WildcardPattern", attrs]
    fn visit_seq<V>(self, mut seq: V) -> Result<Pattern, V::Error>
    where
        V: SeqAccess<'de>,
    {
        let tag: String = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;

        match tag.as_str() {
            "WildcardPattern" | "wildcardPattern" => {
                let attrs: ValueAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Pattern::WildcardPattern(attrs))
            }
            "AsPattern" | "asPattern" => {
                let attrs: ValueAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let pattern: Pattern = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let name: Name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Pattern::AsPattern(attrs, Box::new(pattern), name))
            }
            "TuplePattern" | "tuplePattern" => {
                let attrs: ValueAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let elements: Vec<Pattern> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Pattern::TuplePattern(attrs, elements))
            }
            "ConstructorPattern" | "constructorPattern" => {
                let attrs: ValueAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let name: FQName = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let args: Vec<Pattern> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Pattern::ConstructorPattern(attrs, name, args))
            }
            "EmptyListPattern" | "emptyListPattern" => {
                let attrs: ValueAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Pattern::EmptyListPattern(attrs))
            }
            "HeadTailPattern" | "headTailPattern" => {
                let attrs: ValueAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let head: Pattern = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let tail: Pattern = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Pattern::HeadTailPattern(
                    attrs,
                    Box::new(head),
                    Box::new(tail),
                ))
            }
            "LiteralPattern" | "literalPattern" => {
                let attrs: ValueAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let lit: Literal = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Pattern::LiteralPattern(attrs, lit))
            }
            "UnitPattern" | "unitPattern" => {
                let attrs: ValueAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Pattern::UnitPattern(attrs))
            }
            _ => Err(de::Error::unknown_variant(
                &tag,
                &[
                    "WildcardPattern",
                    "AsPattern",
                    "TuplePattern",
                    "ConstructorPattern",
                    "EmptyListPattern",
                    "HeadTailPattern",
                    "LiteralPattern",
                    "UnitPattern",
                ],
            )),
        }
    }
}

// =============================================================================
// Value Serialization
// =============================================================================

impl Serialize for Value {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Delegate to V4 object wrapper format
        serde_v4::serialize_value(self, serializer)
    }
}

// Note: HoleReason, NativeHint, and NativeInfo serde impls are in value.rs

// =============================================================================
// Tuple Struct Serialization (InputType, RecordFieldEntry, PatternCase, LetBinding, ConstructorArg)
// =============================================================================

// InputType(Name, ValueAttributes, Type) - serialize as [name, attrs, type]
impl Serialize for InputType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(3))?;
        seq.serialize_element(&self.0)?;
        seq.serialize_element(&self.1)?;
        seq.serialize_element(&self.2)?;
        seq.end()
    }
}

impl<'de> Deserialize<'de> for InputType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct InputTypeVisitor;

        impl<'de> Visitor<'de> for InputTypeVisitor {
            type Value = InputType;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a tuple [name, attrs, type]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<InputType, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let attrs = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let tpe = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(InputType(name, attrs, tpe))
            }
        }

        deserializer.deserialize_seq(InputTypeVisitor)
    }
}

// RecordFieldEntry(Name, Value) - serialize as [name, value]
impl Serialize for RecordFieldEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(2))?;
        seq.serialize_element(&self.0)?;
        seq.serialize_element(&self.1)?;
        seq.end()
    }
}

impl<'de> Deserialize<'de> for RecordFieldEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RecordFieldEntryVisitor;

        impl<'de> Visitor<'de> for RecordFieldEntryVisitor {
            type Value = RecordFieldEntry;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a tuple [name, value]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<RecordFieldEntry, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let value = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(RecordFieldEntry(name, value))
            }
        }

        deserializer.deserialize_seq(RecordFieldEntryVisitor)
    }
}

// PatternCase(Pattern, Value) - serialize as [pattern, body]
impl Serialize for PatternCase {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(2))?;
        seq.serialize_element(&self.0)?;
        seq.serialize_element(&self.1)?;
        seq.end()
    }
}

impl<'de> Deserialize<'de> for PatternCase {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PatternCaseVisitor;

        impl<'de> Visitor<'de> for PatternCaseVisitor {
            type Value = PatternCase;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a tuple [pattern, body]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<PatternCase, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let pattern = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let body = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(PatternCase(pattern, body))
            }
        }

        deserializer.deserialize_seq(PatternCaseVisitor)
    }
}

// LetBinding(Name, ValueDefinition) - serialize as [name, definition]
impl Serialize for LetBinding {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(2))?;
        seq.serialize_element(&self.0)?;
        seq.serialize_element(&self.1)?;
        seq.end()
    }
}

impl<'de> Deserialize<'de> for LetBinding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LetBindingVisitor;

        impl<'de> Visitor<'de> for LetBindingVisitor {
            type Value = LetBinding;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a tuple [name, definition]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<LetBinding, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let definition = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(LetBinding(name, definition))
            }
        }

        deserializer.deserialize_seq(LetBindingVisitor)
    }
}

// ConstructorArg(Name, Type) - serialize as [name, type]
impl Serialize for ConstructorArg {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(2))?;
        seq.serialize_element(&self.0)?;
        seq.serialize_element(&self.1)?;
        seq.end()
    }
}

impl<'de> Deserialize<'de> for ConstructorArg {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ConstructorArgVisitor;

        impl<'de> Visitor<'de> for ConstructorArgVisitor {
            type Value = ConstructorArg;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a tuple [name, type]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<ConstructorArg, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let tpe = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(ConstructorArg(name, tpe))
            }
        }

        deserializer.deserialize_seq(ConstructorArgVisitor)
    }
}

// Deserialize for Value (complex, needs visitor)
impl<'de> Deserialize<'de> for Value {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Use deserialize_any to accept V4 objects and Classic arrays
        deserializer.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(
            "V4 object { \"Variable\": { \"name\": \"x\" } } or Classic array [\"Literal\", attrs, lit]",
        )
    }

    /// V4 object wrapper format: { "Variable": { "name": "x" } }
    fn visit_map<M>(self, mut map: M) -> Result<Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        use indexmap::IndexMap;

        let (tag, value): (String, serde_json::Value) = map
            .next_entry()?
            .ok_or_else(|| de::Error::custom("expected object wrapper with single key"))?;

        match tag.as_str() {
            "Literal" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    literal: Literal,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::Literal(attrs, content.literal))
            }
            "Constructor" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    fqname: String,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let fqname = FQName::from_canonical_string(&content.fqname)
                    .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
                Ok(Value::Constructor(attrs, fqname))
            }
            "Tuple" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    elements: Vec<Value>,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::Tuple(attrs, content.elements))
            }
            "List" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    items: Vec<Value>,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::List(attrs, content.items))
            }
            "Record" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    fields: IndexMap<String, Value>,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let fields = content
                    .fields
                    .into_iter()
                    .map(|(name, value)| {
                        Ok(RecordFieldEntry(
                            parse_canonical_name::<M::Error>(&name)?,
                            value,
                        ))
                    })
                    .collect::<Result<Vec<_>, M::Error>>()?;
                Ok(Value::Record(attrs, fields))
            }
            "Variable" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    name: String,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let name = parse_canonical_name::<M::Error>(&content.name)?;
                Ok(Value::Variable(attrs, name))
            }
            "Reference" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    fqname: String,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let fqname = FQName::from_canonical_string(&content.fqname)
                    .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
                Ok(Value::Reference(attrs, fqname))
            }
            "Field" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    value: Value,
                    name: String,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let name = parse_canonical_name::<M::Error>(&content.name)?;
                Ok(Value::Field(attrs, Box::new(content.value), name))
            }
            "FieldFunction" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    name: String,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let name = parse_canonical_name::<M::Error>(&content.name)?;
                Ok(Value::FieldFunction(attrs, name))
            }
            "Apply" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    function: Value,
                    argument: Value,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::Apply(
                    attrs,
                    Box::new(content.function),
                    Box::new(content.argument),
                ))
            }
            "Lambda" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    pattern: Pattern,
                    body: Value,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::Lambda(
                    attrs,
                    content.pattern,
                    Box::new(content.body),
                ))
            }
            "LetDefinition" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    name: String,
                    definition: ValueDefinition,
                    body: Value,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let name = parse_canonical_name::<M::Error>(&content.name)?;
                Ok(Value::LetDefinition(
                    attrs,
                    name,
                    Box::new(content.definition),
                    Box::new(content.body),
                ))
            }
            "LetRecursion" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    bindings: Vec<LetBinding>,
                    body: Value,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::LetRecursion(
                    attrs,
                    content.bindings,
                    Box::new(content.body),
                ))
            }
            "Destructure" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    pattern: Pattern,
                    value: Value,
                    body: Value,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::Destructure(
                    attrs,
                    content.pattern,
                    Box::new(content.value),
                    Box::new(content.body),
                ))
            }
            "IfThenElse" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    condition: Value,
                    then_branch: Value,
                    else_branch: Value,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::IfThenElse(
                    attrs,
                    Box::new(content.condition),
                    Box::new(content.then_branch),
                    Box::new(content.else_branch),
                ))
            }
            "PatternMatch" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    subject: Value,
                    cases: Vec<PatternCase>,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::PatternMatch(
                    attrs,
                    Box::new(content.subject),
                    content.cases,
                ))
            }
            "UpdateRecord" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    record: Value,
                    updates: Vec<RecordFieldEntry>,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::UpdateRecord(
                    attrs,
                    Box::new(content.record),
                    content.updates,
                ))
            }
            "Unit" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::Unit(attrs))
            }
            "Hole" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    reason: HoleReason,
                    tpe: Option<Type>,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::Hole(
                    attrs,
                    content.reason,
                    content.tpe.map(Box::new),
                ))
            }
            "Native" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    fqname: String,
                    info: NativeInfo,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let fqname = FQName::from_canonical_string(&content.fqname)
                    .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
                Ok(Value::Native(attrs, fqname, content.info))
            }
            "External" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    external_name: String,
                    target_platform: String,
                    attrs: Option<ValueAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::External(
                    attrs,
                    content.external_name,
                    content.target_platform,
                ))
            }
            _ => Err(de::Error::unknown_variant(
                &tag,
                &[
                    "Literal",
                    "Constructor",
                    "Tuple",
                    "List",
                    "Record",
                    "Variable",
                    "Reference",
                    "Field",
                    "FieldFunction",
                    "Apply",
                    "Lambda",
                    "LetDefinition",
                    "LetRecursion",
                    "Destructure",
                    "IfThenElse",
                    "PatternMatch",
                    "UpdateRecord",
                    "Unit",
                    "Hole",
                    "Native",
                    "External",
                ],
            )),
        }
    }

    /// Classic tagged array format: ["Literal", attrs, lit]
    fn visit_seq<V>(self, mut seq: V) -> Result<Value, V::Error>
    where
        V: SeqAccess<'de>,
    {
        let tag: String = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;

        match tag.as_str() {
            "Literal" | "literal" => {
                let attrs: ValueAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let lit: Literal = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Value::Literal(attrs, lit))
            }
            "Constructor" | "constructor" => {
                let attrs: ValueAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let fqname: FQName = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Value::Constructor(attrs, fqname))
            }
            "Variable" | "variable" => {
                let attrs: ValueAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let name: Name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Value::Variable(attrs, name))
            }
            "Reference" | "reference" => {
                let attrs: ValueAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let fqname: FQName = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Value::Reference(attrs, fqname))
            }
            "Unit" | "unit" => {
                let attrs: ValueAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Value::Unit(attrs))
            }
            // Other variants handled by V4 object format
            _ => Err(de::Error::unknown_variant(
                &tag,
                &["Literal", "Constructor", "Variable", "Reference", "Unit"],
            )),
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::super::value::NativeHint;
    use super::*;

    #[test]
    fn test_type_serialization_roundtrip() {
        let var = Type::Variable(TypeAttributes::default(), Name::from("a"));
        let json = serde_json::to_string(&var).unwrap();
        let parsed: Type = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, Type::Variable(_, _)));
    }

    #[test]
    fn test_pattern_serialization_roundtrip() {
        let p = Pattern::WildcardPattern(ValueAttributes::default());
        let json = serde_json::to_string(&p).unwrap();
        let parsed: Pattern = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, Pattern::WildcardPattern(_)));
    }

    #[test]
    fn test_pattern_literal_roundtrip() {
        let pattern: Pattern =
            Pattern::LiteralPattern(ValueAttributes::default(), Literal::Integer(42));
        let json = serde_json::to_string(&pattern).unwrap();
        assert!(json.contains("LiteralPattern"));

        let parsed: Pattern = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            parsed,
            Pattern::LiteralPattern(_, Literal::Integer(42))
        ));
    }

    #[test]
    fn test_value_literal_serialization() {
        let val: Value = Value::Literal(ValueAttributes::default(), Literal::Integer(42));
        let json = serde_json::to_string(&val).unwrap();
        assert!(json.contains("Literal"));
        assert!(json.contains("IntegerLiteral"));
    }

    #[test]
    fn test_value_unit_serialization() {
        let val: Value = Value::Unit(ValueAttributes::default());
        let json = serde_json::to_string(&val).unwrap();
        assert!(json.contains("Unit"));
    }

    #[test]
    fn test_value_variable_serialization() {
        let val: Value = Value::Variable(ValueAttributes::default(), Name::from("x"));
        let json = serde_json::to_string(&val).unwrap();
        assert!(json.contains("Variable"));
    }

    #[test]
    fn test_value_apply_serialization() {
        let val: Value = Value::Apply(
            ValueAttributes::default(),
            Box::new(Value::Unit(ValueAttributes::default())),
            Box::new(Value::Unit(ValueAttributes::default())),
        );
        let json = serde_json::to_string(&val).unwrap();
        assert!(json.contains("Apply"));
    }

    #[test]
    fn test_hole_reason_serialization() {
        // Test UnresolvedReference variant
        let target = FQName::from_canonical_string("test:module#func").unwrap();
        let reason = HoleReason::UnresolvedReference { target };
        let json = serde_json::to_string(&reason).unwrap();
        assert!(json.contains("UnresolvedReference"));
        assert!(json.contains("target"));

        let parsed: HoleReason = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, HoleReason::UnresolvedReference { .. }));

        // Test TypeMismatch variant
        let reason2 = HoleReason::TypeMismatch {
            expected: "Int".to_string(),
            found: "String".to_string(),
        };
        let json2 = serde_json::to_string(&reason2).unwrap();
        assert!(json2.contains("TypeMismatch"));

        let parsed2: HoleReason = serde_json::from_str(&json2).unwrap();
        assert!(matches!(parsed2, HoleReason::TypeMismatch { .. }));
    }

    #[test]
    fn test_native_hint_roundtrip() {
        let hint = NativeHint::Arithmetic;
        let json = serde_json::to_string(&hint).unwrap();
        // V4 spec uses wrapper object format
        assert!(json.contains("Arithmetic"));

        let parsed: NativeHint = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, NativeHint::Arithmetic));
    }

    #[test]
    fn test_native_info_roundtrip() {
        let info = NativeInfo {
            hint: NativeHint::StringOp,
            description: Some("String operation".to_string()),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("StringOp"));
        assert!(json.contains("String operation"));

        let parsed: NativeInfo = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed.hint, NativeHint::StringOp));
        assert_eq!(parsed.description, Some("String operation".to_string()));
    }

    #[test]
    fn test_value_serialization_roundtrip() {
        let v = Value::Unit(ValueAttributes::default());
        let json = serde_json::to_string(&v).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, Value::Unit(_)));
    }
}

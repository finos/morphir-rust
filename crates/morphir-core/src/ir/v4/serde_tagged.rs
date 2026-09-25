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
use num_bigint::BigInt;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::fmt;

use super::attributes::{SourceLocation, TypeAttributes, ValueAttributes};
use super::legacy::{accept_member, record_legacy_form_warning};
use super::linked_metadata::MetadataScope;
use super::literal::{FloatLiteral, Literal};
use super::pattern::Pattern;
use super::serde_v4;
use super::types::{Field, Type};
use super::value::{HoleReason, LetBinding, PatternCase, RecordFieldEntry, Value, ValueDefinition};
use crate::ir::decimal::DecimalLiteral;
use crate::ir::{Diagnostic, DiagnosticCode, DiagnosticError};
use crate::naming::{FQName, Name};

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

pub(super) fn carry<E: de::Error>(diagnostic: Diagnostic) -> E {
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

pub(super) fn invalid_type(cursor: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::normalization(DiagnosticCode::InvalidType, cursor, message)
}

fn unknown_node(cursor: &str, seen: &str) -> Diagnostic {
    unknown_node_at(cursor, format!("{seen} is not a type expression"))
}

pub(super) fn unknown_node_at(cursor: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::normalization(DiagnosticCode::UnknownNode, cursor, message)
}

fn invalid_literal(cursor: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::normalization(DiagnosticCode::InvalidLiteral, cursor, message)
}

/// A canonical string that carries both an `:` and a `#` spells an FQName; anything else at a
/// type position spells a type variable's name.
fn looks_like_fqname(text: &str) -> bool {
    text.contains(':') && text.contains('#')
}

/// Whether a single-member object is positioned as a node wrapper.
///
/// The tag is not checked against [`TYPE_TAGS`]: the question here is only whether the author
/// wrote something where a type belongs, and `decode_type` is what decides whether the tag
/// names a node. Keeping the two apart is what lets `{ "Hole": { ... } }` inside a field map be
/// refused as an unknown node at its own cursor, rather than reported as a bad member name one
/// level up.
fn is_wrapper_shaped(members: &serde_json::Map<String, JsonValue>) -> bool {
    members.len() == 1
        && members
            .keys()
            .next()
            .is_some_and(|tag| !spells_a_field_name(tag))
}

/// Whether an object is a Record's field map carried directly, the spelling the schema
/// documented until 2026-09-04.
///
/// Every key must be a canonical `Name` and every value must be type-shaped, or itself a field
/// map by this same rule. Requiring a canonical `Name` is what separates a field map from a
/// wrapper without naming the wrappers: a canonical name admits no mixed-case segment, so no
/// node tag — `Hole` and `Draft` included, not only the seven a `Type` can be — can be mistaken
/// for a field name. `fields`, `attributes` and `attrs` are legal names, so they are excluded
/// explicitly: an object carrying one of them is the expanded spelling, not a field map.
fn is_legacy_field_map(members: &serde_json::Map<String, JsonValue>) -> bool {
    !members.is_empty()
        && !members.contains_key("fields")
        && !members.contains_key("attributes")
        && !members.contains_key("attrs")
        && members.keys().all(|member| spells_a_field_name(member))
        && members.values().all(looks_like_field_type)
}

/// Whether `member` spells a field name, which is a non-empty canonical [`Name`].
fn spells_a_field_name(member: &str) -> bool {
    !member.is_empty() && Name::from_canonical_string(member).is_ok()
}

/// A field's value inside a legacy field map is written where a type belongs, or is another
/// legacy field map: the spelling nests, and each level earns its own warning.
///
/// A structural test keeps this free of side effects: a trial decode would record the nested
/// node's `legacy_spelling` warnings twice.
fn looks_like_field_type(value: &JsonValue) -> bool {
    match value {
        JsonValue::String(_) | JsonValue::Array(_) => true,
        JsonValue::Object(members) => is_wrapper_shaped(members) || is_legacy_field_map(members),
        _ => false,
    }
}

/// Reads a wrapper payload's members, mapping each spelling onto the node's canonical member
/// name through the decision 0006 window table.
pub(super) fn wrapper_members<'a>(
    node: &str,
    payload: &'a JsonValue,
    cursor: &str,
    canonical_members: &[&'static str],
) -> Result<Members<'a>, Diagnostic> {
    let payload = payload
        .as_object()
        .ok_or_else(|| invalid_type(cursor, format!("the {node} payload must be an object")))?;
    wrapper_members_of(node, payload, cursor, canonical_members)
}

/// [`wrapper_members`] for a caller that already holds the payload's members.
///
/// A document-tree file's root is read as a map so the reserved `$meta` can be taken off it before
/// anything else looks at it; handing that map straight in is what keeps the stripped root from
/// having to be wrapped back into a [`JsonValue`] and unwrapped again.
pub(super) fn wrapper_members_of<'a>(
    node: &str,
    payload: &'a serde_json::Map<String, JsonValue>,
    cursor: &str,
    canonical_members: &[&'static str],
) -> Result<Members<'a>, Diagnostic> {
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
pub(super) struct Member<'a> {
    pub(super) value: &'a JsonValue,
    pub(super) seen: &'a str,
}

pub(super) type Members<'a> = IndexMap<&'static str, Member<'a>>;

/// The JSON pointer of `name`, spelled the way the input spelled it.
pub(super) fn member_cursor(members: &Members<'_>, name: &str, cursor: &str) -> String {
    match members.get(name) {
        Some(member) => format!("{cursor}/{}", member.seen),
        None => format!("{cursor}/{name}"),
    }
}

fn decode_attributes(members: &Members<'_>, cursor: &str) -> Result<TypeAttributes, Diagnostic> {
    let Some(member) = members.get("attributes") else {
        return Ok(TypeAttributes::default());
    };
    let at = format!("{cursor}/{}", member.seen);
    let written = wrapper_members(
        "TypeAttributes",
        member.value,
        &at,
        &["source", "constraints", "extensions", "@context", "facts"],
    )?;
    Ok(TypeAttributes {
        metadata: MetadataScope::parse(
            written.get("@context").map(|member| member.value),
            written.get("facts").map(|member| member.value),
        )
        .map_err(|error| invalid_type(&at, error))?,
        source: decode_source(&written, &at)?,
        constraints: decode_object_member(&written, "constraints", &at)?,
        extensions: decode_object_member(&written, "extensions", &at)?,
    })
}

fn decode_source(
    members: &Members<'_>,
    cursor: &str,
) -> Result<Option<SourceLocation>, Diagnostic> {
    match members.get("source") {
        None => Ok(None),
        Some(member) => serde_json::from_value::<SourceLocation>(member.value.clone())
            .map(Some)
            .map_err(|error| {
                invalid_type(&member_cursor(members, "source", cursor), error.to_string())
            }),
    }
}

fn decode_object_member(
    members: &Members<'_>,
    name: &str,
    cursor: &str,
) -> Result<serde_json::Map<String, JsonValue>, Diagnostic> {
    match members.get(name) {
        None => Ok(serde_json::Map::new()),
        Some(member) => member.value.as_object().cloned().ok_or_else(|| {
            invalid_type(
                &member_cursor(members, name, cursor),
                format!("{name} is an object"),
            )
        }),
    }
}

pub(super) fn required<'a>(
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

pub(super) fn decode_name(value: &JsonValue, cursor: &str) -> Result<Name, Diagnostic> {
    let text = value
        .as_str()
        .ok_or_else(|| invalid_type(cursor, "a name must be a canonical string"))?;
    // `Name::from_canonical_string` refuses the empty string itself — a name has at least one
    // segment — so `""` arrives here as an `InvalidName` diagnostic like any other unspellable
    // name, with no separate guard.
    Name::from_canonical_string(text)
        .map_err(|error| Diagnostic::normalization(DiagnosticCode::InvalidName, cursor, error))
}

pub(super) fn decode_fqname(value: &JsonValue, cursor: &str) -> Result<FQName, Diagnostic> {
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
pub(super) fn decode_type(value: &JsonValue, cursor: &str) -> Result<Type, Diagnostic> {
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
// Literal Serialization
// =============================================================================

/// The tag a document literal carries. Its payload is the document itself, so it is the one
/// literal whose payload is never unwrapped.
const DOCUMENT_LITERAL: &str = "DocumentLiteral";

/// The literal tags a v4 reader accepts.
///
/// `WholeNumberLiteral` is the spelling `IntegerLiteral` replaced; it decodes without a warning
/// and is never written.
const LITERAL_TAGS: &[&str] = &[
    "BoolLiteral",
    "CharLiteral",
    "StringLiteral",
    "IntegerLiteral",
    "WholeNumberLiteral",
    "FloatLiteral",
    "DecimalLiteral",
    DOCUMENT_LITERAL,
];

impl<'de> Deserialize<'de> for Literal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // The node is read as JSON first, rather than through a visitor of our own, because
        // serde_json's `Value` is what implements the arbitrary-precision protocol: that is how
        // a document literal's numbers keep the lexeme they were written with.
        let value = JsonValue::deserialize(deserializer)?;
        decode_literal(&value, "").map_err(carry)
    }
}

/// Decodes one literal at `cursor`.
///
/// A literal is a single-member wrapper carrying its value directly. A bare scalar is a literal
/// too, which is what the literal-pattern shorthand rests on.
fn decode_literal(value: &JsonValue, cursor: &str) -> Result<Literal, Diagnostic> {
    match value {
        JsonValue::Object(wrapper) => {
            let mut entries = wrapper.iter();
            let (tag, payload) = entries
                .next()
                .ok_or_else(|| unknown_node_at(cursor, "an empty object is not a literal"))?;
            if entries.next().is_some() {
                return Err(unknown_node_at(
                    cursor,
                    "an object with more than one member is not a literal",
                ));
            }
            decode_literal_wrapper(tag, payload, cursor)
        }
        JsonValue::Bool(carried) => Ok(Literal::Bool(*carried)),
        JsonValue::String(carried) => Ok(Literal::String(carried.clone())),
        JsonValue::Number(_) => decode_number_literal(value, cursor),
        JsonValue::Null | JsonValue::Array(_) => Err(invalid_literal(
            cursor,
            "a literal is a wrapper such as { \"IntegerLiteral\": 42 }, or the value itself",
        )),
    }
}

fn decode_literal_wrapper(
    tag: &str,
    payload: &JsonValue,
    cursor: &str,
) -> Result<Literal, Diagnostic> {
    // A document's payload is the document, so there is no `{ "value": .. }` spelling for it:
    // `{ "DocumentLiteral": { "value": 1 } }` is the one-member document `{ "value": 1 }`.
    if tag == DOCUMENT_LITERAL {
        return Ok(Literal::Document(payload.clone()));
    }
    let at = format!("{cursor}/{tag}");
    let payload = compact_or_expanded(payload);
    match tag {
        "BoolLiteral" => payload
            .as_bool()
            .map(Literal::Bool)
            .ok_or_else(|| invalid_literal(&at, "a BoolLiteral carries true or false")),
        "CharLiteral" => decode_char_literal(payload, &at),
        "StringLiteral" => payload
            .as_str()
            .map(|text| Literal::String(text.to_owned()))
            .ok_or_else(|| invalid_literal(&at, "a StringLiteral carries a string")),
        "IntegerLiteral" | "WholeNumberLiteral" => integer_from_json(payload)
            .map(Literal::Integer)
            .ok_or_else(|| {
                invalid_literal(
                    &at,
                    "an IntegerLiteral carries a whole-number lexeme with no point and no exponent",
                )
            }),
        "FloatLiteral" => float_from_json(payload)
            .map(Literal::Float)
            .ok_or_else(|| invalid_literal(&at, "a FloatLiteral carries a number")),
        // A decimal is a genuine decimal carried as its lexeme (v4 schema page, "Literals").
        "DecimalLiteral" => {
            let text = payload.as_str().ok_or_else(|| {
                invalid_literal(&at, "a DecimalLiteral carries its lexeme as a string")
            })?;
            DecimalLiteral::parse(text).map(Literal::Decimal).map_err(|_| {
                invalid_literal(
                    &at,
                    format!("{text:?} is not a decimal lexeme: [+-]?(digits(.digits?)?|.digits)([eE][+-]?digits)?"),
                )
            })
        }
        _ => Err(unknown_node_at(cursor, format!("{tag} is not a literal"))),
    }
}

/// A literal's value is written directly, or wrapped in a lone `value` member.
fn compact_or_expanded(payload: &JsonValue) -> &JsonValue {
    match payload {
        JsonValue::Object(members) if members.len() == 1 => members.get("value").unwrap_or(payload),
        _ => payload,
    }
}

/// One character means one code point, so an astral character is a single `CharLiteral` and a
/// two-character string is not a character at all.
fn decode_char_literal(payload: &JsonValue, cursor: &str) -> Result<Literal, Diagnostic> {
    let text = payload.as_str().ok_or_else(|| {
        invalid_literal(cursor, "a CharLiteral carries one character as a string")
    })?;
    let mut characters = text.chars();
    match (characters.next(), characters.next()) {
        (Some(only), None) => Ok(Literal::Char(only)),
        _ => Err(invalid_literal(
            cursor,
            "a CharLiteral carries exactly one code point",
        )),
    }
}

/// A bare number is an integer literal when it is a whole number this reader can hold, and a
/// float otherwise.
fn decode_number_literal(value: &JsonValue, cursor: &str) -> Result<Literal, Diagnostic> {
    if let Some(whole) = integer_from_json(value) {
        return Ok(Literal::Integer(whole));
    }
    float_from_json(value)
        .map(Literal::Float)
        .ok_or_else(|| invalid_literal(cursor, "this number is outside the reader's range"))
}

/// Reads a JSON number as an integer literal when its lexeme has no point and no exponent
/// (decision 0009), at any size.
fn integer_from_json(value: &JsonValue) -> Option<BigInt> {
    let lexeme = value.as_number()?.to_string();
    if lexeme.contains(['.', 'e', 'E']) {
        return None;
    }
    BigInt::parse_bytes(lexeme.as_bytes(), 10)
}

/// Reads a JSON number as a float literal, keeping the lexeme it was written with.
///
/// With `arbitrary_precision`, a number parsed from text reports its original spelling from
/// `Number::to_string`, which is how `1.0e2` stays `1.0e2` instead of becoming `100.0`.
fn float_from_json(value: &JsonValue) -> Option<FloatLiteral> {
    let lexeme = value.as_number()?.to_string();
    // `from_lexeme` is the gate: it refuses a spelling that is not a JSON number and a magnitude
    // no `f64` can hold, which is the number the reader cannot carry.
    FloatLiteral::from_lexeme(&lexeme).ok()
}

// =============================================================================
// Pattern Serialization
// =============================================================================

/// The v4 pattern wrapper tags.
///
/// A bare array is a tuple pattern, so a leading element that names one of these is not a
/// pattern in the tuple but a Classic tagged array, which a v4 reader refuses as an unknown
/// node.
const PATTERN_TAGS: &[&str] = &[
    "WildcardPattern",
    "AsPattern",
    "TuplePattern",
    "ConstructorPattern",
    "EmptyListPattern",
    "HeadTailPattern",
    "LiteralPattern",
    "UnitPattern",
];

impl Serialize for Pattern {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serde_v4::serialize_pattern(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Pattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = JsonValue::deserialize(deserializer)?;
        decode_pattern(&value, "").map_err(carry)
    }
}

/// Decodes one pattern at `cursor`.
///
/// A bare array is a tuple pattern and a bare literal is a literal pattern; every other node is
/// a single-member wrapper whose payload may open with `attributes`.
fn decode_pattern(value: &JsonValue, cursor: &str) -> Result<Pattern, Diagnostic> {
    match value {
        JsonValue::Array(items) => {
            if let Some(JsonValue::String(head)) = items.first()
                && (PATTERN_TAGS.contains(&head.as_str()) || LITERAL_TAGS.contains(&head.as_str()))
            {
                return Err(unknown_node_at(cursor, format!("{head} is not a pattern")));
            }
            Ok(Pattern::TuplePattern(
                ValueAttributes::default(),
                decode_pattern_list(value, cursor)?,
            ))
        }
        JsonValue::Object(wrapper) => {
            let mut entries = wrapper.iter();
            let (tag, payload) = entries
                .next()
                .ok_or_else(|| unknown_node_at(cursor, "an empty object is not a pattern"))?;
            if entries.next().is_some() {
                return Err(unknown_node_at(
                    cursor,
                    "an object with more than one member is not a pattern",
                ));
            }
            // A literal written where a pattern belongs is a literal pattern.
            if LITERAL_TAGS.contains(&tag.as_str()) {
                return Ok(Pattern::LiteralPattern(
                    ValueAttributes::default(),
                    pattern_literal(value, cursor)?,
                ));
            }
            decode_pattern_wrapper(tag, payload, cursor)
        }
        JsonValue::Null => Err(invalid_literal(cursor, "null is not a pattern")),
        // A bare literal at a pattern position is a literal pattern.
        scalar => Ok(Pattern::LiteralPattern(
            ValueAttributes::default(),
            decode_literal(scalar, cursor)?,
        )),
    }
}

fn decode_pattern_wrapper(
    tag: &str,
    payload: &JsonValue,
    cursor: &str,
) -> Result<Pattern, Diagnostic> {
    let at = format!("{cursor}/{tag}");
    match tag {
        "WildcardPattern" => Ok(Pattern::WildcardPattern(nullary_attributes(
            tag, payload, &at,
        )?)),
        "EmptyListPattern" => Ok(Pattern::EmptyListPattern(nullary_attributes(
            tag, payload, &at,
        )?)),
        "UnitPattern" => Ok(Pattern::UnitPattern(nullary_attributes(tag, payload, &at)?)),
        "AsPattern" => {
            let members = wrapper_members(tag, payload, &at, &["attributes", "pattern", "name"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let pattern = decode_pattern(
                required(&members, "pattern", &at)?,
                &member_cursor(&members, "pattern", &at),
            )?;
            let name = decode_name(
                required(&members, "name", &at)?,
                &member_cursor(&members, "name", &at),
            )?;
            Ok(Pattern::AsPattern(attributes, Box::new(pattern), name))
        }
        "TuplePattern" => {
            if payload.is_array() {
                return Ok(Pattern::TuplePattern(
                    ValueAttributes::default(),
                    decode_pattern_list(payload, &at)?,
                ));
            }
            let members = wrapper_members(tag, payload, &at, &["attributes", "patterns"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let patterns = decode_pattern_list(
                required(&members, "patterns", &at)?,
                &member_cursor(&members, "patterns", &at),
            )?;
            Ok(Pattern::TuplePattern(attributes, patterns))
        }
        "ConstructorPattern" => {
            let members =
                wrapper_members(tag, payload, &at, &["attributes", "fqname", "patterns"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let fqname = decode_fqname(
                required(&members, "fqname", &at)?,
                &member_cursor(&members, "fqname", &at),
            )?;
            let patterns = match members.get("patterns") {
                None => Vec::new(),
                Some(member) => {
                    decode_pattern_list(member.value, &format!("{at}/{}", member.seen))?
                }
            };
            Ok(Pattern::ConstructorPattern(attributes, fqname, patterns))
        }
        "HeadTailPattern" => {
            let members = wrapper_members(tag, payload, &at, &["attributes", "head", "tail"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let head = decode_pattern(
                required(&members, "head", &at)?,
                &member_cursor(&members, "head", &at),
            )?;
            let tail = decode_pattern(
                required(&members, "tail", &at)?,
                &member_cursor(&members, "tail", &at),
            )?;
            Ok(Pattern::HeadTailPattern(
                attributes,
                Box::new(head),
                Box::new(tail),
            ))
        }
        "LiteralPattern" => {
            // The payload is the literal itself, unless it is the expanded spelling, which
            // carries the literal under `literal` and may open with `attributes`.
            if let JsonValue::Object(written) = payload
                && (written.contains_key("literal")
                    || written.contains_key("attributes")
                    || written.contains_key("attrs"))
            {
                let members = wrapper_members(tag, payload, &at, &["attributes", "literal"])?;
                let attributes = decode_value_attributes(&members, &at)?;
                let literal = pattern_literal(
                    required(&members, "literal", &at)?,
                    &member_cursor(&members, "literal", &at),
                )?;
                return Ok(Pattern::LiteralPattern(attributes, literal));
            }
            Ok(Pattern::LiteralPattern(
                ValueAttributes::default(),
                pattern_literal(payload, &at)?,
            ))
        }
        _ => Err(unknown_node_at(cursor, format!("{tag} is not a pattern"))),
    }
}

/// A wildcard, empty-list or unit pattern takes an empty payload, or `attributes` alone.
fn nullary_attributes(
    tag: &str,
    payload: &JsonValue,
    at: &str,
) -> Result<ValueAttributes, Diagnostic> {
    let members = wrapper_members(tag, payload, at, &["attributes"])?;
    decode_value_attributes(&members, at)
}

fn decode_value_attributes(
    members: &Members<'_>,
    cursor: &str,
) -> Result<ValueAttributes, Diagnostic> {
    let Some(member) = members.get("attributes") else {
        return Ok(ValueAttributes::default());
    };
    let at = format!("{cursor}/{}", member.seen);
    let written = wrapper_members(
        "ValueAttributes",
        member.value,
        &at,
        &["source", "inferredType", "extensions", "@context", "facts"],
    )?;
    let inferred_type = match written.get("inferredType") {
        None => None,
        Some(inferred) => Some(Box::new(decode_type(
            inferred.value,
            &member_cursor(&written, "inferredType", &at),
        )?)),
    };
    Ok(ValueAttributes {
        metadata: MetadataScope::parse(
            written.get("@context").map(|member| member.value),
            written.get("facts").map(|member| member.value),
        )
        .map_err(|error| invalid_type(&at, error))?,
        source: decode_source(&written, &at)?,
        inferred_type,
        extensions: decode_object_member(&written, "extensions", &at)?,
    })
}

fn decode_pattern_list(value: &JsonValue, cursor: &str) -> Result<Vec<Pattern>, Diagnostic> {
    let items = value
        .as_array()
        .ok_or_else(|| invalid_type(cursor, "expected an array of patterns"))?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| decode_pattern(item, &format!("{cursor}/{index}")))
        .collect()
}

/// Reads the literal a pattern matches on.
///
/// Decision 0013: a document is schema-less, so there is nothing for a pattern to match; a
/// `DocumentLiteral` here is refused rather than decoded.
fn pattern_literal(value: &JsonValue, cursor: &str) -> Result<Literal, Diagnostic> {
    if let JsonValue::Object(wrapper) = value
        && wrapper.len() == 1
        && wrapper.contains_key(DOCUMENT_LITERAL)
    {
        return Err(invalid_literal(
            &format!("{cursor}/{DOCUMENT_LITERAL}"),
            "a document cannot be pattern matched",
        ));
    }
    decode_literal(value, cursor)
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
// Tuple Struct Serialization (RecordFieldEntry, PatternCase, LetBinding, ConstructorArg)
// =============================================================================

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

// =============================================================================
// Value Deserialization
// =============================================================================

/// The v4 value wrapper tags.
///
/// A bare array is a List, so a leading element that names one of these is not an item in the
/// list but a Classic tagged array, which a v4 reader refuses as an unknown node.
const VALUE_TAGS: &[&str] = &[
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
];

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Read the node as JSON first, for the reason the literal decode does: serde_json's
        // `Value` is what implements the arbitrary-precision protocol, so a number keeps the
        // lexeme it was written with.
        let value = JsonValue::deserialize(deserializer)?;
        decode_value(&value, "").map_err(carry)
    }
}

/// Decodes one value expression at `cursor`.
///
/// Decision 0009 gives the shorthands: a bare array is a List, a bare number or boolean is a
/// literal, and a bare string is a Variable when it spells a name and a Reference when it spells
/// an FQName. A Tuple always carries its wrapper. Every other node is a single-member wrapper
/// whose payload may open with `attributes`.
pub(super) fn decode_value(value: &JsonValue, cursor: &str) -> Result<Value, Diagnostic> {
    match value {
        JsonValue::String(text) => {
            if looks_like_fqname(text) {
                Ok(Value::Reference(
                    ValueAttributes::default(),
                    decode_fqname(value, cursor)?,
                ))
            } else {
                Ok(Value::Variable(
                    ValueAttributes::default(),
                    decode_name(value, cursor)?,
                ))
            }
        }
        JsonValue::Array(items) => {
            if let Some(JsonValue::String(head)) = items.first()
                && (VALUE_TAGS.contains(&head.as_str()) || LITERAL_TAGS.contains(&head.as_str()))
            {
                return Err(unknown_node_at(
                    cursor,
                    format!("{head} is not a value expression"),
                ));
            }
            Ok(Value::List(
                ValueAttributes::default(),
                decode_value_list(value, cursor)?,
            ))
        }
        JsonValue::Object(wrapper) => {
            let mut entries = wrapper.iter();
            let (tag, payload) = entries.next().ok_or_else(|| {
                unknown_node_at(cursor, "an empty object is not a value expression")
            })?;
            if entries.next().is_some() {
                return Err(unknown_node_at(
                    cursor,
                    "an object with more than one member is not a value expression",
                ));
            }
            decode_value_wrapper(tag, payload, cursor)
        }
        JsonValue::Bool(_) | JsonValue::Number(_) => Ok(Value::Literal(
            ValueAttributes::default(),
            decode_literal(value, cursor)?,
        )),
        JsonValue::Null => Err(invalid_type(cursor, "null is not a value expression")),
    }
}

fn decode_value_wrapper(tag: &str, payload: &JsonValue, cursor: &str) -> Result<Value, Diagnostic> {
    let at = format!("{cursor}/{tag}");
    match tag {
        "Literal" => {
            // The payload is the literal itself, unless it is the expanded spelling, which
            // carries the literal under `literal` and may open with `attributes`.
            if is_expanded_literal(payload) {
                let members = wrapper_members(tag, payload, &at, &["attributes", "literal"])?;
                let attributes = decode_value_attributes(&members, &at)?;
                let literal = decode_literal(
                    required(&members, "literal", &at)?,
                    &member_cursor(&members, "literal", &at),
                )?;
                return Ok(Value::Literal(attributes, literal));
            }
            Ok(Value::Literal(
                ValueAttributes::default(),
                decode_literal(payload, &at)?,
            ))
        }
        "Variable" => {
            if payload.is_string() {
                return Ok(Value::Variable(
                    ValueAttributes::default(),
                    decode_name(payload, &at)?,
                ));
            }
            let members = wrapper_members(tag, payload, &at, &["attributes", "name"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let name = decode_name(
                required(&members, "name", &at)?,
                &member_cursor(&members, "name", &at),
            )?;
            Ok(Value::Variable(attributes, name))
        }
        "FieldFunction" => {
            if payload.is_string() {
                return Ok(Value::FieldFunction(
                    ValueAttributes::default(),
                    decode_name(payload, &at)?,
                ));
            }
            let members = wrapper_members(tag, payload, &at, &["attributes", "name"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let name = decode_name(
                required(&members, "name", &at)?,
                &member_cursor(&members, "name", &at),
            )?;
            Ok(Value::FieldFunction(attributes, name))
        }
        "Reference" | "Constructor" => {
            let (attributes, fqname) = if payload.is_string() {
                (ValueAttributes::default(), decode_fqname(payload, &at)?)
            } else {
                let members = wrapper_members(tag, payload, &at, &["attributes", "fqname"])?;
                let attributes = decode_value_attributes(&members, &at)?;
                let fqname = decode_fqname(
                    required(&members, "fqname", &at)?,
                    &member_cursor(&members, "fqname", &at),
                )?;
                (attributes, fqname)
            };
            Ok(match tag {
                "Reference" => Value::Reference(attributes, fqname),
                _ => Value::Constructor(attributes, fqname),
            })
        }
        "Tuple" => {
            if payload.is_array() {
                return Ok(Value::Tuple(
                    ValueAttributes::default(),
                    decode_value_list(payload, &at)?,
                ));
            }
            let members = wrapper_members(tag, payload, &at, &["attributes", "elements"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let elements = decode_value_list(
                required(&members, "elements", &at)?,
                &member_cursor(&members, "elements", &at),
            )?;
            Ok(Value::Tuple(attributes, elements))
        }
        "List" => {
            if payload.is_array() {
                return Ok(Value::List(
                    ValueAttributes::default(),
                    decode_value_list(payload, &at)?,
                ));
            }
            let members = wrapper_members(tag, payload, &at, &["attributes", "items"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let items = decode_value_list(
                required(&members, "items", &at)?,
                &member_cursor(&members, "items", &at),
            )?;
            Ok(Value::List(attributes, items))
        }
        "Record" => {
            if let JsonValue::Object(written) = payload
                && is_legacy_value_field_map(written)
            {
                let first = written
                    .keys()
                    .next()
                    .expect("a legacy field map is never empty");
                record_legacy_form_warning(&at, first)?;
                return Ok(Value::Record(
                    ValueAttributes::default(),
                    decode_value_fields(payload, &at)?,
                ));
            }
            let members = wrapper_members(tag, payload, &at, &["attributes", "fields"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let fields = decode_value_fields(
                required(&members, "fields", &at)?,
                &member_cursor(&members, "fields", &at),
            )?;
            Ok(Value::Record(attributes, fields))
        }
        "Unit" => {
            let members = wrapper_members(tag, payload, &at, &["attributes"])?;
            Ok(Value::Unit(decode_value_attributes(&members, &at)?))
        }
        "Apply" => {
            let members =
                wrapper_members(tag, payload, &at, &["attributes", "function", "argument"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let function = decode_member_value(&members, "function", &at)?;
            let argument = decode_member_value(&members, "argument", &at)?;
            Ok(Value::Apply(
                attributes,
                Box::new(function),
                Box::new(argument),
            ))
        }
        "IfThenElse" => {
            let members = wrapper_members(
                tag,
                payload,
                &at,
                &["attributes", "condition", "then", "else"],
            )?;
            let attributes = decode_value_attributes(&members, &at)?;
            let condition = decode_member_value(&members, "condition", &at)?;
            let then_branch = decode_member_value(&members, "then", &at)?;
            let else_branch = decode_member_value(&members, "else", &at)?;
            Ok(Value::IfThenElse(
                attributes,
                Box::new(condition),
                Box::new(then_branch),
                Box::new(else_branch),
            ))
        }
        "Field" => {
            let members = wrapper_members(tag, payload, &at, &["attributes", "target", "name"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let target = decode_member_value(&members, "target", &at)?;
            let name = decode_name(
                required(&members, "name", &at)?,
                &member_cursor(&members, "name", &at),
            )?;
            Ok(Value::Field(attributes, Box::new(target), name))
        }
        "Lambda" => {
            let members = wrapper_members(tag, payload, &at, &["attributes", "pattern", "body"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let pattern = decode_pattern(
                required(&members, "pattern", &at)?,
                &member_cursor(&members, "pattern", &at),
            )?;
            let body = decode_member_value(&members, "body", &at)?;
            Ok(Value::Lambda(attributes, pattern, Box::new(body)))
        }
        "LetDefinition" => {
            let members = wrapper_members(
                tag,
                payload,
                &at,
                &["attributes", "name", "definition", "in"],
            )?;
            let attributes = decode_value_attributes(&members, &at)?;
            let name = decode_name(
                required(&members, "name", &at)?,
                &member_cursor(&members, "name", &at),
            )?;
            let definition = decode_value_definition(
                required(&members, "definition", &at)?,
                &member_cursor(&members, "definition", &at),
            )?;
            let body = decode_member_value(&members, "in", &at)?;
            Ok(Value::LetDefinition(
                attributes,
                name,
                Box::new(definition),
                Box::new(body),
            ))
        }
        "LetRecursion" => {
            let members = wrapper_members(tag, payload, &at, &["attributes", "definitions", "in"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let definitions_cursor = member_cursor(&members, "definitions", &at);
            let definitions = required(&members, "definitions", &at)?
                .as_object()
                .ok_or_else(|| {
                    invalid_type(
                        &definitions_cursor,
                        "definitions is an object keyed by the bound name",
                    )
                })?
                .iter()
                .map(|(name, definition)| {
                    let binding_cursor = format!("{definitions_cursor}/{name}");
                    Ok(LetBinding::new(
                        decode_name(&JsonValue::String(name.clone()), &binding_cursor)?,
                        decode_value_definition(definition, &binding_cursor)?,
                    ))
                })
                .collect::<Result<_, Diagnostic>>()?;
            let body = decode_member_value(&members, "in", &at)?;
            Ok(Value::LetRecursion(attributes, definitions, Box::new(body)))
        }
        "Destructure" => {
            let members =
                wrapper_members(tag, payload, &at, &["attributes", "pattern", "value", "in"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let pattern = decode_pattern(
                required(&members, "pattern", &at)?,
                &member_cursor(&members, "pattern", &at),
            )?;
            let value = decode_member_value(&members, "value", &at)?;
            let body = decode_member_value(&members, "in", &at)?;
            Ok(Value::Destructure(
                attributes,
                pattern,
                Box::new(value),
                Box::new(body),
            ))
        }
        "PatternMatch" => {
            let members = wrapper_members(tag, payload, &at, &["attributes", "value", "cases"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let value = decode_member_value(&members, "value", &at)?;
            let cases = decode_pattern_cases(
                required(&members, "cases", &at)?,
                &member_cursor(&members, "cases", &at),
            )?;
            Ok(Value::PatternMatch(attributes, Box::new(value), cases))
        }
        "UpdateRecord" => {
            let members = wrapper_members(tag, payload, &at, &["attributes", "target", "fields"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let target = decode_member_value(&members, "target", &at)?;
            let fields = decode_value_fields(
                required(&members, "fields", &at)?,
                &member_cursor(&members, "fields", &at),
            )?;
            Ok(Value::UpdateRecord(attributes, Box::new(target), fields))
        }
        "Hole" => {
            let members =
                wrapper_members(tag, payload, &at, &["attributes", "reason", "expectedType"])?;
            let attributes = decode_value_attributes(&members, &at)?;
            let reason_cursor = member_cursor(&members, "reason", &at);
            let reason: HoleReason = super::serde_document::decode_hole_reason(
                required(&members, "reason", &at)?,
                &reason_cursor,
            )?;
            let expected = match members.get("expectedType") {
                None => None,
                Some(member) => Some(Box::new(decode_type(
                    member.value,
                    &format!("{at}/{}", member.seen),
                )?)),
            };
            Ok(Value::Hole(attributes, reason, expected))
        }
        // Decision 0008 makes a native operation and an external binding properties of a
        // definition, so `Native` and `External` where a value expression belongs are unknown
        // nodes.
        _ => Err(unknown_node_at(
            cursor,
            format!("{tag} is not a value expression"),
        )),
    }
}

/// Whether a `Literal` wrapper's payload is the expanded spelling rather than the literal itself.
///
/// The three names are the ones the expanded spelling can open with; a literal tag is never one
/// of them.
fn is_expanded_literal(payload: &JsonValue) -> bool {
    match payload {
        JsonValue::Object(written) => {
            written.contains_key("literal")
                || written.contains_key("attributes")
                || written.contains_key("attrs")
        }
        _ => false,
    }
}

/// Whether an object is a Record value's field map carried directly, the spelling decision 0006
/// keeps alive for one release.
///
/// Every key must be a canonical `Name`; the values are not checked, because at a value position
/// anything at all — a bare number, a string, an array — is a legal value expression, so there
/// is nothing a shape test could rule out. `fields`, `attributes` and `attrs` are legal names,
/// so they are excluded explicitly: an object carrying one of them is the expanded spelling.
fn is_legacy_value_field_map(members: &serde_json::Map<String, JsonValue>) -> bool {
    !members.is_empty()
        && !members.contains_key("fields")
        && !members.contains_key("attributes")
        && !members.contains_key("attrs")
        && members.keys().all(|member| spells_a_field_name(member))
}

fn decode_value_list(value: &JsonValue, cursor: &str) -> Result<Vec<Value>, Diagnostic> {
    let items = value
        .as_array()
        .ok_or_else(|| invalid_type(cursor, "expected an array of value expressions"))?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| decode_value(item, &format!("{cursor}/{index}")))
        .collect()
}

/// Decodes a field map at a value position: an object keyed by field name, whose order is the
/// field order.
fn decode_value_fields(
    value: &JsonValue,
    cursor: &str,
) -> Result<Vec<RecordFieldEntry>, Diagnostic> {
    let fields = value
        .as_object()
        .ok_or_else(|| invalid_type(cursor, "fields must be an object keyed by field name"))?;
    fields
        .iter()
        .map(|(name, field)| {
            let field_cursor = format!("{cursor}/{name}");
            Ok(RecordFieldEntry(
                decode_name(&JsonValue::String(name.clone()), &field_cursor)?,
                decode_value(field, &field_cursor)?,
            ))
        })
        .collect()
}

/// Decodes a required member that carries a value expression, reporting it at the spelling the
/// input used.
fn decode_member_value(
    members: &Members<'_>,
    name: &str,
    cursor: &str,
) -> Result<Value, Diagnostic> {
    decode_value(
        required(members, name, cursor)?,
        &member_cursor(members, name, cursor),
    )
}

/// Decodes a pattern match's cases: an array of `{ "pattern": …, "body": … }` objects.
fn decode_pattern_cases(value: &JsonValue, cursor: &str) -> Result<Vec<PatternCase>, Diagnostic> {
    let cases = value
        .as_array()
        .ok_or_else(|| invalid_type(cursor, "cases is an array of pattern and body pairs"))?;
    cases
        .iter()
        .enumerate()
        .map(|(index, case)| {
            let at = format!("{cursor}/{index}");
            let members = wrapper_members("PatternMatchCase", case, &at, &["pattern", "body"])?;
            Ok(PatternCase(
                decode_pattern(
                    required(&members, "pattern", &at)?,
                    &member_cursor(&members, "pattern", &at),
                )?,
                decode_value(
                    required(&members, "body", &at)?,
                    &member_cursor(&members, "body", &at),
                )?,
            ))
        })
        .collect()
}

/// Decodes a value definition nested inside a value expression, carrying the cursor of the
/// member that holds it so a diagnostic inside a let definition's body reports its full path.
fn decode_value_definition(value: &JsonValue, cursor: &str) -> Result<ValueDefinition, Diagnostic> {
    super::serde_document::decode_value_definition(value, cursor)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::super::value::{NativeHint, NativeInfo};
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
            Pattern::LiteralPattern(ValueAttributes::default(), Literal::Integer(42.into()));
        let json = serde_json::to_string(&pattern).unwrap();
        assert!(json.contains("LiteralPattern"));

        let parsed: Pattern = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            parsed,
            Pattern::LiteralPattern(_, Literal::Integer(n)) if n == BigInt::from(42)
        ));
    }

    #[test]
    fn test_value_literal_serialization() {
        let val: Value = Value::Literal(ValueAttributes::default(), Literal::Integer(42.into()));
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

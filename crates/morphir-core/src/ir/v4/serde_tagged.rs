//! Serde implementations for Morphir IR types.
//!
//! Serialization uses V4 object wrapper format:
//! - `{ "Variable": { "name": "a" } }`
//! - `{ "Reference": { "fqname": "morphir/sdk:basics#int" } }`
//! - `{ "Tuple": { "elements": [...] } }`
//!
//! Deserialization accepts both V4 and Classic formats for backward compatibility:
//! - V4 object: `{ "Variable": { "name": "a" } }`
//! - Classic array: `["Variable", attrs, name]`
//! - V4 shorthand: `"a"` (variable) or `"morphir/sdk:basics#int"` (reference)

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};
use std::fmt;

use super::attributes::{TypeAttributes, ValueAttributes};
use super::literal::Literal;
use super::pattern::Pattern;
use super::serde_v4;
use super::type_def::ConstructorArg;
use super::types::{Field, Type};
use super::value::{
    HoleReason, InputType, LetBinding, NativeInfo, PatternCase, RecordFieldEntry, Value,
    ValueDefinition,
};
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
        // Use deserialize_any to accept V4 objects, Classic arrays, and string shorthand
        deserializer.deserialize_any(TypeVisitor)
    }
}

struct TypeVisitor;

impl<'de> Visitor<'de> for TypeVisitor {
    type Value = Type;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(
            "V4 object { \"Variable\": { \"name\": \"a\" } }, \
             Classic array [\"Variable\", attrs, name], \
             or string shorthand \"a\"",
        )
    }

    /// V4 object wrapper format: { "Variable": { "name": "a" } }
    fn visit_map<M>(self, mut map: M) -> Result<Type, M::Error>
    where
        M: MapAccess<'de>,
    {
        use indexmap::IndexMap;

        let (tag, value): (String, serde_json::Value) = map
            .next_entry()?
            .ok_or_else(|| de::Error::custom("expected object wrapper with single key"))?;

        match tag.as_str() {
            "Variable" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    name: String,
                    attrs: Option<TypeAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let name = Name::from(content.name.as_str());
                let attrs = content.attrs.unwrap_or_default();
                Ok(Type::Variable(attrs, name))
            }
            "Reference" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    fqname: String,
                    args: Option<Vec<Type>>,
                    attrs: Option<TypeAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let fqname = FQName::from_canonical_string(&content.fqname)
                    .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
                let attrs = content.attrs.unwrap_or_default();
                let args = content.args.unwrap_or_default();
                Ok(Type::Reference(attrs, fqname, args))
            }
            "Tuple" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    elements: Vec<Type>,
                    attrs: Option<TypeAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Type::Tuple(attrs, content.elements))
            }
            "Record" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    fields: IndexMap<String, Type>,
                    attrs: Option<TypeAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let fields = content
                    .fields
                    .into_iter()
                    .map(|(name, tpe)| Field {
                        name: Name::from(name.as_str()),
                        tpe,
                    })
                    .collect();
                Ok(Type::Record(attrs, fields))
            }
            "ExtensibleRecord" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    variable: String,
                    fields: IndexMap<String, Type>,
                    attrs: Option<TypeAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let variable = Name::from(content.variable.as_str());
                let fields = content
                    .fields
                    .into_iter()
                    .map(|(name, tpe)| Field {
                        name: Name::from(name.as_str()),
                        tpe,
                    })
                    .collect();
                Ok(Type::ExtensibleRecord(attrs, variable, fields))
            }
            "Function" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    arg: Type,
                    result: Type,
                    attrs: Option<TypeAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Type::Function(
                    attrs,
                    Box::new(content.arg),
                    Box::new(content.result),
                ))
            }
            "Unit" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content {
                    attrs: Option<TypeAttributes>,
                }
                let content: Content = serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Type::Unit(attrs))
            }
            _ => Err(de::Error::unknown_variant(
                &tag,
                &[
                    "Variable",
                    "Reference",
                    "Tuple",
                    "Record",
                    "ExtensibleRecord",
                    "Function",
                    "Unit",
                ],
            )),
        }
    }

    /// Classic tagged array format: ["Variable", attrs, name]
    fn visit_seq<V>(self, mut seq: V) -> Result<Type, V::Error>
    where
        V: SeqAccess<'de>,
    {
        let tag: String = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;

        match tag.as_str() {
            "Variable" | "variable" => {
                let attrs: TypeAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let name: Name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Type::Variable(attrs, name))
            }
            "Reference" | "reference" => {
                let attrs: TypeAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let fqname: FQName = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let params: Vec<Type> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Type::Reference(attrs, fqname, params))
            }
            "Tuple" | "tuple" => {
                let attrs: TypeAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let elements: Vec<Type> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Type::Tuple(attrs, elements))
            }
            "Record" | "record" => {
                let attrs: TypeAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let fields: Vec<Field> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Type::Record(attrs, fields))
            }
            "ExtensibleRecord" | "extensibleRecord" => {
                let attrs: TypeAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let var: Name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let fields: Vec<Field> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Type::ExtensibleRecord(attrs, var, fields))
            }
            "Function" | "function" => {
                let attrs: TypeAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let arg: Type = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let result: Type = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Type::Function(attrs, Box::new(arg), Box::new(result)))
            }
            "Unit" | "unit" => {
                let attrs: TypeAttributes = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Type::Unit(attrs))
            }
            _ => Err(de::Error::unknown_variant(
                &tag,
                &[
                    "Variable",
                    "Reference",
                    "Tuple",
                    "Record",
                    "ExtensibleRecord",
                    "Function",
                    "Unit",
                ],
            )),
        }
    }

    /// V4 string shorthand: "a" for Variable, "morphir/sdk:basics#int" for Reference
    fn visit_str<E>(self, v: &str) -> Result<Type, E>
    where
        E: de::Error,
    {
        if v.contains(':') && v.contains('#') {
            // FQName shorthand for Reference
            let fqname = FQName::from_canonical_string(v)
                .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
            Ok(Type::Reference(TypeAttributes::default(), fqname, vec![]))
        } else {
            // Variable shorthand
            let name = Name::from(v);
            Ok(Type::Variable(TypeAttributes::default(), name))
        }
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
                let name = Name::from(content.name.as_str());
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
                    .map(|(name, val)| RecordFieldEntry(Name::from(name.as_str()), val))
                    .collect();
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
                let name = Name::from(content.name.as_str());
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
                let name = Name::from(content.name.as_str());
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
                let name = Name::from(content.name.as_str());
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
                let name = Name::from(content.name.as_str());
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

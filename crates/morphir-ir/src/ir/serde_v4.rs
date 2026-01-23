//! V4 object wrapper serialization for Morphir IR.
//!
//! V4 uses object wrapper format for all expressions:
//! - `{ "Variable": { "name": "a" } }` instead of `["Variable", {}, ["a"]]`
//! - `{ "Reference": { "fqname": "morphir/sdk:basics#int" } }` instead of `["Reference", {}, ...]`
//!
//! This module provides Serialize/Deserialize implementations for Type, Pattern, Value,
//! and Literal using the V4 object wrapper format.

use indexmap::IndexMap;
use serde::de::{self, DeserializeOwned, Deserializer, MapAccess, Visitor};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::marker::PhantomData;

use super::literal::Literal;
use super::type_expr::{Field, Type};
use crate::naming::{FQName, Name};

// =============================================================================
// Type<A> V4 Serialization
// =============================================================================

/// V4 serialization module for Type<A>
pub mod type_serde {
    use super::*;

    /// Serialize Type in V4 object wrapper format
    pub fn serialize<A, S>(tpe: &Type<A>, serializer: S) -> Result<S::Ok, S::Error>
    where
        A: Clone + Serialize,
        S: Serializer,
    {
        serialize_type(tpe, serializer)
    }

    /// Deserialize Type from V4 object wrapper format (also accepts Classic format)
    pub fn deserialize<'de, A, D>(deserializer: D) -> Result<Type<A>, D::Error>
    where
        A: Clone + Default + DeserializeOwned,
        D: Deserializer<'de>,
    {
        deserialize_type(deserializer)
    }
}

/// Serialize a Type in V4 object wrapper format
pub fn serialize_type<A, S>(tpe: &Type<A>, serializer: S) -> Result<S::Ok, S::Error>
where
    A: Clone + Serialize,
    S: Serializer,
{
    match tpe {
        Type::Variable(attrs, name) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Variable",
                &VariableContent {
                    name: name.to_string(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Type::Reference(attrs, fqname, args) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Reference",
                &ReferenceContent {
                    fqname: fqname.to_canonical_string(),
                    args,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Type::Tuple(attrs, elements) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Tuple",
                &TupleContent {
                    elements,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Type::Record(attrs, fields) => {
            let mut map = serializer.serialize_map(Some(1))?;
            // V4 Record fields are object: { "fieldName": typeExpr }
            let fields_map: IndexMap<String, &Type<A>> = fields
                .iter()
                .map(|f| (f.name.to_string(), &f.tpe))
                .collect();
            map.serialize_entry(
                "Record",
                &RecordContent {
                    fields: fields_map,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Type::ExtensibleRecord(attrs, var, fields) => {
            let mut map = serializer.serialize_map(Some(1))?;
            let fields_map: IndexMap<String, &Type<A>> = fields
                .iter()
                .map(|f| (f.name.to_string(), &f.tpe))
                .collect();
            map.serialize_entry(
                "ExtensibleRecord",
                &ExtensibleRecordContent {
                    variable: var.to_string(),
                    fields: fields_map,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Type::Function(attrs, arg, result) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Function",
                &FunctionContent {
                    arg: arg.as_ref(),
                    result: result.as_ref(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Type::Unit(attrs) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("Unit", &UnitContent { attrs: Some(attrs) })?;
            map.end()
        }
    }
}

// Helper structs for V4 Type serialization

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VariableContent<'a, A> {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a A>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceContent<'a, A: Clone + Serialize> {
    fqname: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    args: &'a Vec<Type<A>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a A>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TupleContent<'a, A: Clone + Serialize> {
    elements: &'a Vec<Type<A>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a A>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordContent<'a, A: Clone + Serialize> {
    fields: IndexMap<String, &'a Type<A>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a A>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensibleRecordContent<'a, A: Clone + Serialize> {
    variable: String,
    fields: IndexMap<String, &'a Type<A>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a A>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FunctionContent<'a, A: Clone + Serialize> {
    arg: &'a Type<A>,
    result: &'a Type<A>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a A>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UnitContent<'a, A> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a A>,
}

/// Deserialize Type from V4 object wrapper format
/// Also accepts Classic tagged array format for backward compatibility
pub fn deserialize_type<'de, A, D>(deserializer: D) -> Result<Type<A>, D::Error>
where
    A: Clone + Default + DeserializeOwned,
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(TypeV4Visitor(PhantomData))
}

struct TypeV4Visitor<A>(PhantomData<A>);

impl<'de, A: Clone + Default + DeserializeOwned> Visitor<'de> for TypeV4Visitor<A> {
    type Value = Type<A>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a V4 object wrapper like { \"Variable\": { \"name\": \"a\" } } or Classic tagged array")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Type<A>, M::Error>
    where
        M: MapAccess<'de>,
    {
        // V4 format: { "Variable": { "name": "a" } }
        let (tag, value): (String, serde_json::Value) = map
            .next_entry()?
            .ok_or_else(|| de::Error::custom("expected object wrapper with single key"))?;

        match tag.as_str() {
            "Variable" => {
                let content: VariableDeContent<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let name = Name::from(content.name.as_str());
                let attrs = content.attrs.unwrap_or_else(|| {
                    // Create default attrs - this requires A to be Default or we use a workaround
                    A::default()
                });
                Ok(Type::Variable(attrs, name))
            }
            "Reference" => {
                let content: ReferenceDeContent<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let fqname = FQName::from_canonical_string(&content.fqname)
                    .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
                let attrs = content.attrs.unwrap_or_else(|| A::default());
                let args = content.args.unwrap_or_default();
                Ok(Type::Reference(attrs, fqname, args))
            }
            "Tuple" => {
                let content: TupleDeContent<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(|| A::default());
                Ok(Type::Tuple(attrs, content.elements))
            }
            "Record" => {
                let content: RecordDeContent<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(|| A::default());
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
                let content: ExtensibleRecordDeContent<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(|| A::default());
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
                let content: FunctionDeContent<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(|| A::default());
                Ok(Type::Function(
                    attrs,
                    Box::new(content.arg),
                    Box::new(content.result),
                ))
            }
            "Unit" => {
                let content: UnitDeContent<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(|| A::default());
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

    fn visit_seq<V>(self, mut seq: V) -> Result<Type<A>, V::Error>
    where
        V: de::SeqAccess<'de>,
    {
        // Classic format: ["Variable", attrs, name]
        // Delegate to the classic deserializer
        let tag: String = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;

        match tag.as_str() {
            "Variable" | "variable" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let name: Name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Type::Variable(attrs, name))
            }
            "Reference" | "reference" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let fqname: FQName = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let params: Vec<Type<A>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Type::Reference(attrs, fqname, params))
            }
            "Tuple" | "tuple" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let elements: Vec<Type<A>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Type::Tuple(attrs, elements))
            }
            "Record" | "record" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let fields: Vec<Field<A>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Type::Record(attrs, fields))
            }
            "ExtensibleRecord" | "extensibleRecord" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let var: Name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let fields: Vec<Field<A>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Type::ExtensibleRecord(attrs, var, fields))
            }
            "Function" | "function" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let arg: Type<A> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let result: Type<A> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Type::Function(attrs, Box::new(arg), Box::new(result)))
            }
            "Unit" | "unit" => {
                let attrs: A = seq
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

    fn visit_str<E>(self, v: &str) -> Result<Type<A>, E>
    where
        E: de::Error,
    {
        // V4 shorthand: "a" for Variable, "morphir/sdk:basics#int" for Reference
        if v.contains(':') && v.contains('#') {
            // FQName shorthand for Reference
            let fqname = FQName::from_canonical_string(v)
                .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
            Ok(Type::Reference(A::default(), fqname, vec![]))
        } else {
            // Variable shorthand
            let name = Name::from(v);
            Ok(Type::Variable(A::default(), name))
        }
    }
}

// Helper structs for V4 Type deserialization

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VariableDeContent<A> {
    name: String,
    attrs: Option<A>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceDeContent<A: Clone> {
    fqname: String,
    args: Option<Vec<Type<A>>>,
    attrs: Option<A>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TupleDeContent<A: Clone> {
    elements: Vec<Type<A>>,
    attrs: Option<A>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordDeContent<A: Clone> {
    fields: IndexMap<String, Type<A>>,
    attrs: Option<A>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtensibleRecordDeContent<A: Clone> {
    variable: String,
    fields: IndexMap<String, Type<A>>,
    attrs: Option<A>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FunctionDeContent<A: Clone> {
    arg: Type<A>,
    result: Type<A>,
    attrs: Option<A>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnitDeContent<A> {
    attrs: Option<A>,
}

// =============================================================================
// Literal V4 Serialization
// =============================================================================

/// Serialize Literal in V4 object wrapper format
pub fn serialize_literal<S>(lit: &Literal, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match lit {
        Literal::Bool(v) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("BoolLiteral", &LiteralValue { value: v })?;
            map.end()
        }
        Literal::Char(v) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "CharLiteral",
                &LiteralValue {
                    value: v.to_string(),
                },
            )?;
            map.end()
        }
        Literal::String(v) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("StringLiteral", &LiteralValue { value: v })?;
            map.end()
        }
        Literal::Integer(v) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("IntegerLiteral", &LiteralValue { value: v })?;
            map.end()
        }
        Literal::Float(v) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("FloatLiteral", &LiteralValue { value: v })?;
            map.end()
        }
        Literal::Decimal(v) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("DecimalLiteral", &LiteralValue { value: v })?;
            map.end()
        }
    }
}

#[derive(Serialize)]
struct LiteralValue<T: Serialize> {
    value: T,
}

/// Deserialize Literal from V4 object wrapper format
/// Also accepts Classic tagged array format for backward compatibility
pub fn deserialize_literal<'de, D>(deserializer: D) -> Result<Literal, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(LiteralV4Visitor)
}

struct LiteralV4Visitor;

impl<'de> Visitor<'de> for LiteralV4Visitor {
    type Value = Literal;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(
            "a V4 object wrapper like { \"IntegerLiteral\": { \"value\": 42 } } or Classic array",
        )
    }

    fn visit_map<M>(self, mut map: M) -> Result<Literal, M::Error>
    where
        M: MapAccess<'de>,
    {
        let (tag, value): (String, serde_json::Value) = map
            .next_entry()?
            .ok_or_else(|| de::Error::custom("expected object wrapper with single key"))?;

        match tag.as_str() {
            "BoolLiteral" => {
                let content: LiteralValueDe<bool> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(Literal::Bool(content.value))
            }
            "CharLiteral" => {
                let content: LiteralValueDe<String> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let c = content
                    .value
                    .chars()
                    .next()
                    .ok_or_else(|| de::Error::custom("empty char literal"))?;
                Ok(Literal::Char(c))
            }
            "StringLiteral" => {
                let content: LiteralValueDe<String> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(Literal::String(content.value))
            }
            "IntegerLiteral" | "WholeNumberLiteral" => {
                let content: LiteralValueDe<i64> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(Literal::Integer(content.value))
            }
            "FloatLiteral" => {
                let content: LiteralValueDe<f64> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(Literal::Float(content.value))
            }
            "DecimalLiteral" => {
                let content: LiteralValueDe<String> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(Literal::Decimal(content.value))
            }
            _ => Err(de::Error::unknown_variant(
                &tag,
                &[
                    "BoolLiteral",
                    "CharLiteral",
                    "StringLiteral",
                    "IntegerLiteral",
                    "WholeNumberLiteral",
                    "FloatLiteral",
                    "DecimalLiteral",
                ],
            )),
        }
    }

    fn visit_seq<V>(self, mut seq: V) -> Result<Literal, V::Error>
    where
        V: de::SeqAccess<'de>,
    {
        // Classic format: ["IntegerLiteral", 42]
        let tag: String = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;

        match tag.as_str() {
            "BoolLiteral" => {
                let value: bool = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Literal::Bool(value))
            }
            "CharLiteral" => {
                let value: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let c = value
                    .chars()
                    .next()
                    .ok_or_else(|| de::Error::custom("empty char literal"))?;
                Ok(Literal::Char(c))
            }
            "StringLiteral" => {
                let value: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Literal::String(value))
            }
            "IntegerLiteral" | "WholeNumberLiteral" => {
                let value: i64 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Literal::Integer(value))
            }
            "FloatLiteral" => {
                let value: f64 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Literal::Float(value))
            }
            "DecimalLiteral" => {
                let value: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Literal::Decimal(value))
            }
            _ => Err(de::Error::unknown_variant(
                &tag,
                &[
                    "BoolLiteral",
                    "CharLiteral",
                    "StringLiteral",
                    "IntegerLiteral",
                    "WholeNumberLiteral",
                    "FloatLiteral",
                    "DecimalLiteral",
                ],
            )),
        }
    }
}

#[derive(Deserialize)]
struct LiteralValueDe<T> {
    value: T,
}

// =============================================================================
// Pattern<A> V4 Serialization
// =============================================================================

use super::pattern::Pattern;

/// V4 serialization module for Pattern<A>
pub mod pattern_serde {
    use super::*;

    /// Serialize Pattern in V4 object wrapper format
    pub fn serialize<A, S>(pat: &Pattern<A>, serializer: S) -> Result<S::Ok, S::Error>
    where
        A: Clone + Serialize,
        S: Serializer,
    {
        serialize_pattern(pat, serializer)
    }

    /// Deserialize Pattern from V4 object wrapper format (also accepts Classic format)
    pub fn deserialize<'de, A, D>(deserializer: D) -> Result<Pattern<A>, D::Error>
    where
        A: Clone + Default + DeserializeOwned,
        D: Deserializer<'de>,
    {
        deserialize_pattern(deserializer)
    }
}

/// Serialize a Pattern in V4 object wrapper format
pub fn serialize_pattern<A, S>(pat: &Pattern<A>, serializer: S) -> Result<S::Ok, S::Error>
where
    A: Clone + Serialize,
    S: Serializer,
{
    match pat {
        Pattern::WildcardPattern(attrs) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "WildcardPattern",
                &PatternAttrsContent { attrs: Some(attrs) },
            )?;
            map.end()
        }
        Pattern::AsPattern(attrs, pattern, name) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "AsPattern",
                &AsPatternContent {
                    pattern,
                    name: name.to_string(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Pattern::TuplePattern(attrs, elements) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "TuplePattern",
                &TuplePatternContent {
                    elements,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Pattern::ConstructorPattern(attrs, fqname, args) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "ConstructorPattern",
                &ConstructorPatternContent {
                    fqname: fqname.to_canonical_string(),
                    args,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Pattern::EmptyListPattern(attrs) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "EmptyListPattern",
                &PatternAttrsContent { attrs: Some(attrs) },
            )?;
            map.end()
        }
        Pattern::HeadTailPattern(attrs, head, tail) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "HeadTailPattern",
                &HeadTailPatternContent {
                    head: head.as_ref(),
                    tail: tail.as_ref(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Pattern::LiteralPattern(attrs, lit) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "LiteralPattern",
                &LiteralPatternContent {
                    literal: lit,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Pattern::UnitPattern(attrs) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("UnitPattern", &PatternAttrsContent { attrs: Some(attrs) })?;
            map.end()
        }
    }
}

// Helper structs for V4 Pattern serialization

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PatternAttrsContent<'a, A> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a A>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AsPatternContent<'a, A: Clone + Serialize> {
    #[serde(serialize_with = "serialize_pattern")]
    pattern: &'a Pattern<A>,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a A>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TuplePatternContent<'a, A: Clone + Serialize> {
    elements: &'a Vec<Pattern<A>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a A>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConstructorPatternContent<'a, A: Clone + Serialize> {
    fqname: String,
    args: &'a Vec<Pattern<A>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a A>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HeadTailPatternContent<'a, A: Clone + Serialize> {
    #[serde(serialize_with = "serialize_pattern")]
    head: &'a Pattern<A>,
    #[serde(serialize_with = "serialize_pattern")]
    tail: &'a Pattern<A>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a A>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LiteralPatternContent<'a, A> {
    #[serde(serialize_with = "serialize_literal")]
    literal: &'a Literal,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a A>,
}

/// Deserialize Pattern from V4 object wrapper format
/// Also accepts Classic tagged array format for backward compatibility
pub fn deserialize_pattern<'de, A, D>(deserializer: D) -> Result<Pattern<A>, D::Error>
where
    A: Clone + Default + DeserializeOwned,
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(PatternV4Visitor(PhantomData))
}

struct PatternV4Visitor<A>(PhantomData<A>);

impl<'de, A: Clone + Default + DeserializeOwned> Visitor<'de> for PatternV4Visitor<A> {
    type Value = Pattern<A>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(
            "a V4 object wrapper like { \"WildcardPattern\": {} } or Classic tagged array",
        )
    }

    fn visit_map<M>(self, mut map: M) -> Result<Pattern<A>, M::Error>
    where
        M: MapAccess<'de>,
    {
        let (tag, value): (String, serde_json::Value) = map
            .next_entry()?
            .ok_or_else(|| de::Error::custom("expected object wrapper with single key"))?;

        match tag.as_str() {
            "WildcardPattern" => {
                let content: PatternAttrsDeContent<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(A::default);
                Ok(Pattern::WildcardPattern(attrs))
            }
            "AsPattern" => {
                let content: AsPatternDeContent<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(A::default);
                let name = Name::from(content.name.as_str());
                Ok(Pattern::AsPattern(attrs, Box::new(content.pattern), name))
            }
            "TuplePattern" => {
                let content: TuplePatternDeContent<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(A::default);
                Ok(Pattern::TuplePattern(attrs, content.elements))
            }
            "ConstructorPattern" => {
                let content: ConstructorPatternDeContent<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(A::default);
                let fqname = FQName::from_canonical_string(&content.fqname)
                    .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
                Ok(Pattern::ConstructorPattern(attrs, fqname, content.args))
            }
            "EmptyListPattern" => {
                let content: PatternAttrsDeContent<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(A::default);
                Ok(Pattern::EmptyListPattern(attrs))
            }
            "HeadTailPattern" => {
                let content: HeadTailPatternDeContent<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(A::default);
                Ok(Pattern::HeadTailPattern(
                    attrs,
                    Box::new(content.head),
                    Box::new(content.tail),
                ))
            }
            "LiteralPattern" => {
                let content: LiteralPatternDeContent<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(A::default);
                Ok(Pattern::LiteralPattern(attrs, content.literal))
            }
            "UnitPattern" => {
                let content: PatternAttrsDeContent<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(A::default);
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

    fn visit_seq<V>(self, mut seq: V) -> Result<Pattern<A>, V::Error>
    where
        V: de::SeqAccess<'de>,
    {
        // Classic format: ["WildcardPattern", attrs]
        let tag: String = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;

        match tag.as_str() {
            "WildcardPattern" | "wildcardPattern" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Pattern::WildcardPattern(attrs))
            }
            "AsPattern" | "asPattern" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let pattern: Pattern<A> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let name: Name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Pattern::AsPattern(attrs, Box::new(pattern), name))
            }
            "TuplePattern" | "tuplePattern" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let elements: Vec<Pattern<A>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Pattern::TuplePattern(attrs, elements))
            }
            "ConstructorPattern" | "constructorPattern" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let fqname: FQName = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let args: Vec<Pattern<A>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Pattern::ConstructorPattern(attrs, fqname, args))
            }
            "EmptyListPattern" | "emptyListPattern" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Pattern::EmptyListPattern(attrs))
            }
            "HeadTailPattern" | "headTailPattern" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let head: Pattern<A> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let tail: Pattern<A> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Pattern::HeadTailPattern(
                    attrs,
                    Box::new(head),
                    Box::new(tail),
                ))
            }
            "LiteralPattern" | "literalPattern" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let literal: Literal = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Pattern::LiteralPattern(attrs, literal))
            }
            "UnitPattern" | "unitPattern" => {
                let attrs: A = seq
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

// Helper structs for V4 Pattern deserialization
// Note: We avoid using serde derive with DeserializeOwned by using serde(bound) explicitly

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "A: Clone + Default + DeserializeOwned"))]
struct PatternAttrsDeContent<A: Clone> {
    attrs: Option<A>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "A: Clone + Default + DeserializeOwned"))]
struct AsPatternDeContent<A: Clone> {
    #[serde(deserialize_with = "deserialize_pattern")]
    pattern: Pattern<A>,
    name: String,
    attrs: Option<A>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "A: Clone + Default + DeserializeOwned"))]
struct TuplePatternDeContent<A: Clone> {
    elements: Vec<Pattern<A>>,
    attrs: Option<A>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "A: Clone + Default + DeserializeOwned"))]
struct ConstructorPatternDeContent<A: Clone> {
    fqname: String,
    args: Vec<Pattern<A>>,
    attrs: Option<A>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "A: Clone + Default + DeserializeOwned"))]
struct HeadTailPatternDeContent<A: Clone> {
    #[serde(deserialize_with = "deserialize_pattern")]
    head: Pattern<A>,
    #[serde(deserialize_with = "deserialize_pattern")]
    tail: Pattern<A>,
    attrs: Option<A>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "A: Clone + Default + DeserializeOwned"))]
struct LiteralPatternDeContent<A: Clone> {
    #[serde(deserialize_with = "deserialize_literal")]
    literal: Literal,
    attrs: Option<A>,
}

// =============================================================================
// HoleReason, NativeHint, NativeInfo V4 Serialization
// =============================================================================

use super::value_expr::{
    HoleReason, LetBinding, NativeHint, NativeInfo, PatternCase, RecordFieldEntry, Value,
    ValueBody, ValueDefinition,
};

/// Serialize HoleReason in V4 object wrapper format
pub fn serialize_hole_reason<S>(reason: &HoleReason, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match reason {
        HoleReason::Draft => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("Draft", &serde_json::json!({}))?;
            map.end()
        }
        HoleReason::TypeMismatch => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("TypeMismatch", &serde_json::json!({}))?;
            map.end()
        }
        HoleReason::DeletedDuringRefactor => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("DeletedDuringRefactor", &serde_json::json!({}))?;
            map.end()
        }
        HoleReason::UnresolvedReference { target } => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "UnresolvedReference",
                &serde_json::json!({ "target": target.to_canonical_string() }),
            )?;
            map.end()
        }
    }
}

/// Deserialize HoleReason from V4 object wrapper format
pub fn deserialize_hole_reason<'de, D>(deserializer: D) -> Result<HoleReason, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match &value {
        // V4 object wrapper format: { "Draft": {} }
        serde_json::Value::Object(map) => {
            if let Some((key, _)) = map.iter().next() {
                match key.as_str() {
                    "Draft" => Ok(HoleReason::Draft),
                    "TypeMismatch" => Ok(HoleReason::TypeMismatch),
                    "DeletedDuringRefactor" => Ok(HoleReason::DeletedDuringRefactor),
                    "UnresolvedReference" => {
                        let content = map.get("UnresolvedReference").ok_or_else(|| {
                            de::Error::custom("missing UnresolvedReference content")
                        })?;
                        let target_str = content
                            .get("target")
                            .and_then(|t| t.as_str())
                            .ok_or_else(|| de::Error::missing_field("target"))?;
                        let target =
                            FQName::from_canonical_string(target_str).map_err(de::Error::custom)?;
                        Ok(HoleReason::UnresolvedReference { target })
                    }
                    _ => Err(de::Error::unknown_variant(
                        key,
                        &[
                            "Draft",
                            "TypeMismatch",
                            "DeletedDuringRefactor",
                            "UnresolvedReference",
                        ],
                    )),
                }
            } else {
                Err(de::Error::custom("empty object for HoleReason"))
            }
        }
        // Also accept string format for backward compatibility
        serde_json::Value::String(s) => match s.as_str() {
            "Draft" => Ok(HoleReason::Draft),
            "TypeMismatch" => Ok(HoleReason::TypeMismatch),
            "DeletedDuringRefactor" => Ok(HoleReason::DeletedDuringRefactor),
            _ => Err(de::Error::unknown_variant(
                s,
                &["Draft", "TypeMismatch", "DeletedDuringRefactor"],
            )),
        },
        _ => Err(de::Error::custom(
            "expected object or string for HoleReason",
        )),
    }
}

/// Serialize NativeHint in V4 object wrapper format
pub fn serialize_native_hint<S>(hint: &NativeHint, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut map = serializer.serialize_map(Some(1))?;
    match hint {
        NativeHint::Arithmetic => map.serialize_entry("Arithmetic", &serde_json::json!({}))?,
        NativeHint::Comparison => map.serialize_entry("Comparison", &serde_json::json!({}))?,
        NativeHint::StringOp => map.serialize_entry("StringOp", &serde_json::json!({}))?,
        NativeHint::CollectionOp => map.serialize_entry("CollectionOp", &serde_json::json!({}))?,
        NativeHint::PlatformSpecific => {
            map.serialize_entry("PlatformSpecific", &serde_json::json!({}))?
        }
    }
    map.end()
}

/// Deserialize NativeHint from V4 object wrapper format
pub fn deserialize_native_hint<'de, D>(deserializer: D) -> Result<NativeHint, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match &value {
        // V4 object wrapper format: { "Arithmetic": {} }
        serde_json::Value::Object(map) => {
            if let Some((key, _)) = map.iter().next() {
                match key.as_str() {
                    "Arithmetic" => Ok(NativeHint::Arithmetic),
                    "Comparison" => Ok(NativeHint::Comparison),
                    "StringOp" => Ok(NativeHint::StringOp),
                    "CollectionOp" => Ok(NativeHint::CollectionOp),
                    "PlatformSpecific" => Ok(NativeHint::PlatformSpecific),
                    _ => Err(de::Error::unknown_variant(
                        key,
                        &[
                            "Arithmetic",
                            "Comparison",
                            "StringOp",
                            "CollectionOp",
                            "PlatformSpecific",
                        ],
                    )),
                }
            } else {
                Err(de::Error::custom("empty object for NativeHint"))
            }
        }
        // Also accept string format for backward compatibility
        serde_json::Value::String(s) => match s.as_str() {
            "Arithmetic" => Ok(NativeHint::Arithmetic),
            "Comparison" => Ok(NativeHint::Comparison),
            "StringOp" => Ok(NativeHint::StringOp),
            "CollectionOp" => Ok(NativeHint::CollectionOp),
            "PlatformSpecific" => Ok(NativeHint::PlatformSpecific),
            _ => Err(de::Error::unknown_variant(
                s,
                &[
                    "Arithmetic",
                    "Comparison",
                    "StringOp",
                    "CollectionOp",
                    "PlatformSpecific",
                ],
            )),
        },
        _ => Err(de::Error::custom(
            "expected object or string for NativeHint",
        )),
    }
}

/// Serialize NativeInfo in V4 format
pub fn serialize_native_info<S>(info: &NativeInfo, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    use serde::ser::SerializeStruct;
    let mut state = serializer.serialize_struct("NativeInfo", 2)?;
    // Serialize hint using V4 object wrapper format
    state.serialize_field("hint", &NativeHintWrapper(&info.hint))?;
    state.serialize_field("description", &info.description)?;
    state.end()
}

struct NativeHintWrapper<'a>(&'a NativeHint);

impl<'a> Serialize for NativeHintWrapper<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_native_hint(self.0, serializer)
    }
}

/// Serialize ValueBody in V4 object wrapper format
pub fn serialize_value_body<TA, VA, S>(
    body: &ValueBody<TA, VA>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    TA: Clone + Serialize,
    VA: Clone + Serialize,
    S: Serializer,
{
    match body {
        ValueBody::Expression(val) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("ExpressionBody", &ExpressionBodyContent { body: val })?;
            map.end()
        }
        ValueBody::Native(info) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("NativeBody", &NativeBodyContent { info })?;
            map.end()
        }
        ValueBody::External {
            external_name,
            target_platform,
        } => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "ExternalBody",
                &ExternalBodyContent {
                    external_name: external_name.clone(),
                    target_platform: target_platform.clone(),
                },
            )?;
            map.end()
        }
        ValueBody::Incomplete(reason) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("IncompleteBody", &IncompleteBodyContent { reason })?;
            map.end()
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpressionBodyContent<'a, TA: Clone + Serialize, VA: Clone + Serialize> {
    #[serde(serialize_with = "serialize_value")]
    body: &'a Value<TA, VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeBodyContent<'a> {
    #[serde(serialize_with = "serialize_native_info")]
    info: &'a NativeInfo,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalBodyContent {
    external_name: String,
    target_platform: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IncompleteBodyContent<'a> {
    #[serde(serialize_with = "serialize_hole_reason")]
    reason: &'a HoleReason,
}

/// Deserialize ValueBody from V4 object wrapper format
pub fn deserialize_value_body<'de, TA, VA, D>(
    deserializer: D,
) -> Result<ValueBody<TA, VA>, D::Error>
where
    TA: Clone + Default + DeserializeOwned,
    VA: Clone + Default + DeserializeOwned,
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match &value {
        serde_json::Value::Object(map) => {
            if let Some((key, content)) = map.iter().next() {
                match key.as_str() {
                    "ExpressionBody" => {
                        let body_val = content
                            .get("body")
                            .ok_or_else(|| de::Error::missing_field("body"))?;
                        let body: Value<TA, VA> =
                            serde_json::from_value(body_val.clone()).map_err(de::Error::custom)?;
                        Ok(ValueBody::Expression(body))
                    }
                    "NativeBody" => {
                        let info: NativeInfo =
                            serde_json::from_value(content.clone()).map_err(de::Error::custom)?;
                        Ok(ValueBody::Native(info))
                    }
                    "ExternalBody" => {
                        let external_name = content
                            .get("externalName")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| de::Error::missing_field("externalName"))?
                            .to_string();
                        let target_platform = content
                            .get("targetPlatform")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| de::Error::missing_field("targetPlatform"))?
                            .to_string();
                        Ok(ValueBody::External {
                            external_name,
                            target_platform,
                        })
                    }
                    "IncompleteBody" => {
                        let reason_val = content
                            .get("reason")
                            .ok_or_else(|| de::Error::missing_field("reason"))?;
                        let reason: HoleReason = serde_json::from_value(reason_val.clone())
                            .map_err(de::Error::custom)?;
                        Ok(ValueBody::Incomplete(reason))
                    }
                    // Also accept Classic format with "kind" field
                    _ if map.contains_key("kind") => {
                        let kind = map
                            .get("kind")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| de::Error::missing_field("kind"))?;
                        match kind {
                            "Expression" => {
                                let body_val = map
                                    .get("value")
                                    .ok_or_else(|| de::Error::missing_field("value"))?;
                                let body: Value<TA, VA> = serde_json::from_value(body_val.clone())
                                    .map_err(de::Error::custom)?;
                                Ok(ValueBody::Expression(body))
                            }
                            "Native" => {
                                let info_val = map
                                    .get("info")
                                    .ok_or_else(|| de::Error::missing_field("info"))?;
                                let info: NativeInfo = serde_json::from_value(info_val.clone())
                                    .map_err(de::Error::custom)?;
                                Ok(ValueBody::Native(info))
                            }
                            "External" => {
                                let external_name = map
                                    .get("externalName")
                                    .and_then(|v| v.as_str())
                                    .ok_or_else(|| de::Error::missing_field("externalName"))?
                                    .to_string();
                                let target_platform = map
                                    .get("targetPlatform")
                                    .and_then(|v| v.as_str())
                                    .ok_or_else(|| de::Error::missing_field("targetPlatform"))?
                                    .to_string();
                                Ok(ValueBody::External {
                                    external_name,
                                    target_platform,
                                })
                            }
                            "Incomplete" => {
                                let reason_val = map
                                    .get("reason")
                                    .ok_or_else(|| de::Error::missing_field("reason"))?;
                                let reason: HoleReason = serde_json::from_value(reason_val.clone())
                                    .map_err(de::Error::custom)?;
                                Ok(ValueBody::Incomplete(reason))
                            }
                            _ => Err(de::Error::unknown_variant(
                                kind,
                                &["Expression", "Native", "External", "Incomplete"],
                            )),
                        }
                    }
                    _ => Err(de::Error::unknown_variant(
                        key,
                        &[
                            "ExpressionBody",
                            "NativeBody",
                            "ExternalBody",
                            "IncompleteBody",
                        ],
                    )),
                }
            } else {
                Err(de::Error::custom("empty object for ValueBody"))
            }
        }
        _ => Err(de::Error::custom("expected object for ValueBody")),
    }
}

// =============================================================================
// Value<TA, VA> V4 Serialization
// =============================================================================

/// V4 serialization module for Value<TA, VA>
pub mod value_serde {
    use super::*;

    /// Serialize Value in V4 object wrapper format
    pub fn serialize<TA, VA, S>(val: &Value<TA, VA>, serializer: S) -> Result<S::Ok, S::Error>
    where
        TA: Clone + Serialize,
        VA: Clone + Serialize,
        S: Serializer,
    {
        serialize_value(val, serializer)
    }

    /// Deserialize Value from V4 object wrapper format (also accepts Classic format)
    pub fn deserialize<'de, TA, VA, D>(deserializer: D) -> Result<Value<TA, VA>, D::Error>
    where
        TA: Clone + Default + DeserializeOwned,
        VA: Clone + Default + DeserializeOwned,
        D: Deserializer<'de>,
    {
        deserialize_value(deserializer)
    }
}

/// Serialize a Value in V4 object wrapper format
pub fn serialize_value<TA, VA, S>(val: &Value<TA, VA>, serializer: S) -> Result<S::Ok, S::Error>
where
    TA: Clone + Serialize,
    VA: Clone + Serialize,
    S: Serializer,
{
    match val {
        Value::Literal(attrs, lit) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Literal",
                &LiteralValueContent {
                    literal: lit,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Constructor(attrs, fqname) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Constructor",
                &ConstructorValueContent {
                    fqname: fqname.to_canonical_string(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Tuple(attrs, elements) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Tuple",
                &TupleValueContent {
                    elements,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::List(attrs, items) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "List",
                &ListValueContent {
                    items,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Record(attrs, fields) => {
            let mut map = serializer.serialize_map(Some(1))?;
            let fields_map: IndexMap<String, &Value<TA, VA>> =
                fields.iter().map(|f| (f.0.to_string(), &f.1)).collect();
            map.serialize_entry(
                "Record",
                &RecordValueContent {
                    fields: fields_map,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Variable(attrs, name) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Variable",
                &VariableValueContent {
                    name: name.to_string(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Reference(attrs, fqname) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Reference",
                &ReferenceValueContent {
                    fqname: fqname.to_canonical_string(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Field(attrs, value, name) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Field",
                &FieldValueContent {
                    value: value.as_ref(),
                    name: name.to_string(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::FieldFunction(attrs, name) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "FieldFunction",
                &FieldFunctionValueContent {
                    name: name.to_string(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Apply(attrs, function, argument) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Apply",
                &ApplyValueContent {
                    function: function.as_ref(),
                    argument: argument.as_ref(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Lambda(attrs, pattern, body) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Lambda",
                &LambdaValueContent {
                    argument_pattern: pattern,
                    body: body.as_ref(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::LetDefinition(attrs, name, definition, body) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "LetDefinition",
                &LetDefinitionValueContent {
                    name: name.to_string(),
                    definition: definition.as_ref(),
                    body: body.as_ref(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::LetRecursion(attrs, bindings, body) => {
            let mut map = serializer.serialize_map(Some(1))?;
            let bindings_map: IndexMap<String, &ValueDefinition<TA, VA>> =
                bindings.iter().map(|b| (b.0.to_string(), &b.1)).collect();
            map.serialize_entry(
                "LetRecursion",
                &LetRecursionValueContent {
                    bindings: bindings_map,
                    body: body.as_ref(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Destructure(attrs, pattern, value, body) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Destructure",
                &DestructureValueContent {
                    pattern,
                    value: value.as_ref(),
                    body: body.as_ref(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::IfThenElse(attrs, condition, then_branch, else_branch) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "IfThenElse",
                &IfThenElseValueContent {
                    condition: condition.as_ref(),
                    then_branch: then_branch.as_ref(),
                    else_branch: else_branch.as_ref(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::PatternMatch(attrs, value, cases) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "PatternMatch",
                &PatternMatchValueContent {
                    value: value.as_ref(),
                    cases,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::UpdateRecord(attrs, value, fields) => {
            let mut map = serializer.serialize_map(Some(1))?;
            let fields_map: IndexMap<String, &Value<TA, VA>> =
                fields.iter().map(|f| (f.0.to_string(), &f.1)).collect();
            map.serialize_entry(
                "UpdateRecord",
                &UpdateRecordValueContent {
                    value: value.as_ref(),
                    fields: fields_map,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Unit(attrs) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("Unit", &ValueAttrsContent { attrs: Some(attrs) })?;
            map.end()
        }
        // V4-only variants
        Value::Hole(attrs, reason, expected_type) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Hole",
                &HoleValueContent {
                    reason,
                    expected_type: expected_type.as_ref().map(|t| t.as_ref()),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Native(attrs, fqname, info) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Native",
                &NativeValueContent {
                    fqname: fqname.to_canonical_string(),
                    info,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::External(attrs, external_name, target_platform) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "External",
                &ExternalValueContent {
                    external_name: external_name.clone(),
                    target_platform: target_platform.clone(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
    }
}

// Helper structs for V4 Value serialization

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ValueAttrsContent<'a, VA> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LiteralValueContent<'a, VA> {
    #[serde(serialize_with = "serialize_literal")]
    literal: &'a Literal,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConstructorValueContent<'a, VA> {
    fqname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TupleValueContent<'a, TA: Clone + Serialize, VA: Clone + Serialize> {
    elements: &'a Vec<Value<TA, VA>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListValueContent<'a, TA: Clone + Serialize, VA: Clone + Serialize> {
    items: &'a Vec<Value<TA, VA>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordValueContent<'a, TA: Clone + Serialize, VA: Clone + Serialize> {
    fields: IndexMap<String, &'a Value<TA, VA>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VariableValueContent<'a, VA> {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceValueContent<'a, VA> {
    fqname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldValueContent<'a, TA: Clone + Serialize, VA: Clone + Serialize> {
    #[serde(serialize_with = "serialize_value")]
    value: &'a Value<TA, VA>,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldFunctionValueContent<'a, VA> {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApplyValueContent<'a, TA: Clone + Serialize, VA: Clone + Serialize> {
    #[serde(serialize_with = "serialize_value")]
    function: &'a Value<TA, VA>,
    #[serde(serialize_with = "serialize_value")]
    argument: &'a Value<TA, VA>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LambdaValueContent<'a, TA: Clone + Serialize, VA: Clone + Serialize> {
    #[serde(serialize_with = "serialize_pattern")]
    argument_pattern: &'a Pattern<VA>,
    #[serde(serialize_with = "serialize_value")]
    body: &'a Value<TA, VA>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LetDefinitionValueContent<'a, TA: Clone + Serialize, VA: Clone + Serialize> {
    name: String,
    definition: &'a ValueDefinition<TA, VA>,
    #[serde(serialize_with = "serialize_value")]
    body: &'a Value<TA, VA>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LetRecursionValueContent<'a, TA: Clone + Serialize, VA: Clone + Serialize> {
    bindings: IndexMap<String, &'a ValueDefinition<TA, VA>>,
    #[serde(serialize_with = "serialize_value")]
    body: &'a Value<TA, VA>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DestructureValueContent<'a, TA: Clone + Serialize, VA: Clone + Serialize> {
    #[serde(serialize_with = "serialize_pattern")]
    pattern: &'a Pattern<VA>,
    #[serde(serialize_with = "serialize_value")]
    value: &'a Value<TA, VA>,
    #[serde(serialize_with = "serialize_value")]
    body: &'a Value<TA, VA>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IfThenElseValueContent<'a, TA: Clone + Serialize, VA: Clone + Serialize> {
    #[serde(serialize_with = "serialize_value")]
    condition: &'a Value<TA, VA>,
    #[serde(serialize_with = "serialize_value")]
    then_branch: &'a Value<TA, VA>,
    #[serde(serialize_with = "serialize_value")]
    else_branch: &'a Value<TA, VA>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PatternMatchValueContent<'a, TA: Clone + Serialize, VA: Clone + Serialize> {
    #[serde(serialize_with = "serialize_value")]
    value: &'a Value<TA, VA>,
    cases: &'a Vec<PatternCase<TA, VA>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateRecordValueContent<'a, TA: Clone + Serialize, VA: Clone + Serialize> {
    #[serde(serialize_with = "serialize_value")]
    value: &'a Value<TA, VA>,
    fields: IndexMap<String, &'a Value<TA, VA>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HoleValueContent<'a, TA: Clone + Serialize, VA> {
    reason: &'a HoleReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_type: Option<&'a Type<TA>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeValueContent<'a, VA> {
    fqname: String,
    info: &'a NativeInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalValueContent<'a, VA> {
    external_name: String,
    target_platform: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a VA>,
}

/// Deserialize Value from V4 object wrapper format
/// Also accepts Classic tagged array format for backward compatibility
pub fn deserialize_value<'de, TA, VA, D>(deserializer: D) -> Result<Value<TA, VA>, D::Error>
where
    TA: Clone + Default + DeserializeOwned,
    VA: Clone + Default + DeserializeOwned,
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(ValueV4Visitor(PhantomData))
}

struct ValueV4Visitor<TA, VA>(PhantomData<(TA, VA)>);

impl<'de, TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned>
    Visitor<'de> for ValueV4Visitor<TA, VA>
{
    type Value = Value<TA, VA>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a V4 object wrapper like { \"Variable\": { \"name\": \"x\" } } or Classic tagged array")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Value<TA, VA>, M::Error>
    where
        M: MapAccess<'de>,
    {
        let (tag, value): (String, serde_json::Value) = map
            .next_entry()?
            .ok_or_else(|| de::Error::custom("expected object wrapper with single key"))?;

        match tag.as_str() {
            "Literal" => {
                let content: LiteralValueDeContent<VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                Ok(Value::Literal(attrs, content.literal))
            }
            "Constructor" => {
                let content: ConstructorValueDeContent<VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                let fqname = FQName::from_canonical_string(&content.fqname)
                    .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
                Ok(Value::Constructor(attrs, fqname))
            }
            "Tuple" => {
                let content: TupleValueDeContent<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                Ok(Value::Tuple(attrs, content.elements))
            }
            "List" => {
                let content: ListValueDeContent<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                Ok(Value::List(attrs, content.items))
            }
            "Record" => {
                let content: RecordValueDeContent<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                let fields = content
                    .fields
                    .into_iter()
                    .map(|(name, val)| RecordFieldEntry(Name::from(name.as_str()), val))
                    .collect();
                Ok(Value::Record(attrs, fields))
            }
            "Variable" => {
                let content: VariableValueDeContent<VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                let name = Name::from(content.name.as_str());
                Ok(Value::Variable(attrs, name))
            }
            "Reference" => {
                let content: ReferenceValueDeContent<VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                let fqname = FQName::from_canonical_string(&content.fqname)
                    .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
                Ok(Value::Reference(attrs, fqname))
            }
            "Field" => {
                let content: FieldValueDeContent<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                let name = Name::from(content.name.as_str());
                Ok(Value::Field(attrs, Box::new(content.value), name))
            }
            "FieldFunction" => {
                let content: FieldFunctionValueDeContent<VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                let name = Name::from(content.name.as_str());
                Ok(Value::FieldFunction(attrs, name))
            }
            "Apply" => {
                let content: ApplyValueDeContent<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                Ok(Value::Apply(
                    attrs,
                    Box::new(content.function),
                    Box::new(content.argument),
                ))
            }
            "Lambda" => {
                let content: LambdaValueDeContent<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                Ok(Value::Lambda(
                    attrs,
                    content.argument_pattern,
                    Box::new(content.body),
                ))
            }
            "LetDefinition" => {
                let content: LetDefinitionValueDeContent<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                let name = Name::from(content.name.as_str());
                Ok(Value::LetDefinition(
                    attrs,
                    name,
                    Box::new(content.definition),
                    Box::new(content.body),
                ))
            }
            "LetRecursion" => {
                let content: LetRecursionValueDeContent<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                let bindings = content
                    .bindings
                    .into_iter()
                    .map(|(name, def)| LetBinding(Name::from(name.as_str()), def))
                    .collect();
                Ok(Value::LetRecursion(attrs, bindings, Box::new(content.body)))
            }
            "Destructure" => {
                let content: DestructureValueDeContent<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                Ok(Value::Destructure(
                    attrs,
                    content.pattern,
                    Box::new(content.value),
                    Box::new(content.body),
                ))
            }
            "IfThenElse" => {
                let content: IfThenElseValueDeContent<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                Ok(Value::IfThenElse(
                    attrs,
                    Box::new(content.condition),
                    Box::new(content.then_branch),
                    Box::new(content.else_branch),
                ))
            }
            "PatternMatch" => {
                let content: PatternMatchValueDeContent<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                Ok(Value::PatternMatch(
                    attrs,
                    Box::new(content.value),
                    content.cases,
                ))
            }
            "UpdateRecord" => {
                let content: UpdateRecordValueDeContent<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                let fields = content
                    .fields
                    .into_iter()
                    .map(|(name, val)| RecordFieldEntry(Name::from(name.as_str()), val))
                    .collect();
                Ok(Value::UpdateRecord(attrs, Box::new(content.value), fields))
            }
            "Unit" => {
                let content: ValueAttrsDeContent<VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                Ok(Value::Unit(attrs))
            }
            // V4-only variants
            "Hole" => {
                let content: HoleValueDeContent<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                Ok(Value::Hole(
                    attrs,
                    content.reason,
                    content.expected_type.map(Box::new),
                ))
            }
            "Native" => {
                let content: NativeValueDeContent<VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
                let fqname = FQName::from_canonical_string(&content.fqname)
                    .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
                Ok(Value::Native(attrs, fqname, content.info))
            }
            "External" => {
                let content: ExternalValueDeContent<VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_else(VA::default);
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
}

// Helper structs for V4 Value deserialization

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "VA: Clone + Default + DeserializeOwned"))]
struct ValueAttrsDeContent<VA: Clone> {
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "VA: Clone + Default + DeserializeOwned"))]
struct LiteralValueDeContent<VA: Clone> {
    #[serde(deserialize_with = "deserialize_literal")]
    literal: Literal,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "VA: Clone + Default + DeserializeOwned"))]
struct ConstructorValueDeContent<VA: Clone> {
    fqname: String,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(
    deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"
))]
struct TupleValueDeContent<TA: Clone, VA: Clone> {
    elements: Vec<Value<TA, VA>>,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(
    deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"
))]
struct ListValueDeContent<TA: Clone, VA: Clone> {
    items: Vec<Value<TA, VA>>,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(
    deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"
))]
struct RecordValueDeContent<TA: Clone, VA: Clone> {
    fields: IndexMap<String, Value<TA, VA>>,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "VA: Clone + Default + DeserializeOwned"))]
struct VariableValueDeContent<VA: Clone> {
    name: String,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "VA: Clone + Default + DeserializeOwned"))]
struct ReferenceValueDeContent<VA: Clone> {
    fqname: String,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(
    deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"
))]
struct FieldValueDeContent<TA: Clone, VA: Clone> {
    #[serde(deserialize_with = "deserialize_value")]
    value: Value<TA, VA>,
    name: String,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "VA: Clone + Default + DeserializeOwned"))]
struct FieldFunctionValueDeContent<VA: Clone> {
    name: String,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(
    deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"
))]
struct ApplyValueDeContent<TA: Clone, VA: Clone> {
    #[serde(deserialize_with = "deserialize_value")]
    function: Value<TA, VA>,
    #[serde(deserialize_with = "deserialize_value")]
    argument: Value<TA, VA>,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(
    deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"
))]
struct LambdaValueDeContent<TA: Clone, VA: Clone> {
    #[serde(deserialize_with = "deserialize_pattern")]
    argument_pattern: Pattern<VA>,
    #[serde(deserialize_with = "deserialize_value")]
    body: Value<TA, VA>,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(
    deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"
))]
struct LetDefinitionValueDeContent<TA: Clone, VA: Clone> {
    name: String,
    definition: ValueDefinition<TA, VA>,
    #[serde(deserialize_with = "deserialize_value")]
    body: Value<TA, VA>,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(
    deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"
))]
struct LetRecursionValueDeContent<TA: Clone, VA: Clone> {
    bindings: IndexMap<String, ValueDefinition<TA, VA>>,
    #[serde(deserialize_with = "deserialize_value")]
    body: Value<TA, VA>,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(
    deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"
))]
struct DestructureValueDeContent<TA: Clone, VA: Clone> {
    #[serde(deserialize_with = "deserialize_pattern")]
    pattern: Pattern<VA>,
    #[serde(deserialize_with = "deserialize_value")]
    value: Value<TA, VA>,
    #[serde(deserialize_with = "deserialize_value")]
    body: Value<TA, VA>,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(
    deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"
))]
struct IfThenElseValueDeContent<TA: Clone, VA: Clone> {
    #[serde(deserialize_with = "deserialize_value")]
    condition: Value<TA, VA>,
    #[serde(deserialize_with = "deserialize_value")]
    then_branch: Value<TA, VA>,
    #[serde(deserialize_with = "deserialize_value")]
    else_branch: Value<TA, VA>,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(
    deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"
))]
struct PatternMatchValueDeContent<TA: Clone, VA: Clone> {
    #[serde(deserialize_with = "deserialize_value")]
    value: Value<TA, VA>,
    cases: Vec<PatternCase<TA, VA>>,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(
    deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"
))]
struct UpdateRecordValueDeContent<TA: Clone, VA: Clone> {
    #[serde(deserialize_with = "deserialize_value")]
    value: Value<TA, VA>,
    fields: IndexMap<String, Value<TA, VA>>,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(
    deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"
))]
struct HoleValueDeContent<TA: Clone, VA: Clone> {
    reason: HoleReason,
    expected_type: Option<Type<TA>>,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "VA: Clone + Default + DeserializeOwned"))]
struct NativeValueDeContent<VA: Clone> {
    fqname: String,
    info: NativeInfo,
    attrs: Option<VA>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "VA: Clone + Default + DeserializeOwned"))]
struct ExternalValueDeContent<VA: Clone> {
    external_name: String,
    target_platform: String,
    attrs: Option<VA>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::TypeAttributes;

    #[test]
    fn test_type_variable_v4_serialization() {
        let var: Type<TypeAttributes> = Type::Variable(TypeAttributes::default(), Name::from("a"));

        let json = serde_json::to_string(&TypeWrapper(&var)).unwrap();
        assert!(json.contains("Variable"));
        assert!(json.contains("name"));
        assert!(json.contains("\"a\""));
    }

    #[test]
    fn test_type_unit_v4_serialization() {
        let unit: Type<TypeAttributes> = Type::Unit(TypeAttributes::default());
        let json = serde_json::to_string(&TypeWrapper(&unit)).unwrap();
        assert!(json.contains("Unit"));
    }

    #[test]
    fn test_literal_integer_v4_serialization() {
        let lit = Literal::Integer(42);
        let json = serde_json::to_string(&LiteralWrapper(&lit)).unwrap();
        assert!(json.contains("IntegerLiteral"));
        assert!(json.contains("value"));
        assert!(json.contains("42"));
    }

    #[test]
    fn test_literal_string_v4_serialization() {
        let lit = Literal::String("hello".to_string());
        let json = serde_json::to_string(&LiteralWrapper(&lit)).unwrap();
        assert!(json.contains("StringLiteral"));
        assert!(json.contains("hello"));
    }

    // Wrapper structs for testing serialization
    struct TypeWrapper<'a, A: Clone + Serialize>(&'a Type<A>);

    impl<'a, A: Clone + Serialize> Serialize for TypeWrapper<'a, A> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serialize_type(self.0, serializer)
        }
    }

    struct LiteralWrapper<'a>(&'a Literal);

    impl<'a> Serialize for LiteralWrapper<'a> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serialize_literal(self.0, serializer)
        }
    }

    struct PatternWrapper<'a, A: Clone + Serialize>(&'a Pattern<A>);

    impl<'a, A: Clone + Serialize> Serialize for PatternWrapper<'a, A> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serialize_pattern(self.0, serializer)
        }
    }

    #[test]
    fn test_pattern_wildcard_v4_serialization() {
        use crate::ir::ValueAttributes;
        let pat: Pattern<ValueAttributes> = Pattern::WildcardPattern(ValueAttributes::default());
        let json = serde_json::to_string(&PatternWrapper(&pat)).unwrap();
        assert!(json.contains("WildcardPattern"));
    }

    #[test]
    fn test_pattern_tuple_v4_serialization() {
        use crate::ir::ValueAttributes;
        let pat: Pattern<ValueAttributes> = Pattern::TuplePattern(
            ValueAttributes::default(),
            vec![
                Pattern::WildcardPattern(ValueAttributes::default()),
                Pattern::WildcardPattern(ValueAttributes::default()),
            ],
        );
        let json = serde_json::to_string(&PatternWrapper(&pat)).unwrap();
        assert!(json.contains("TuplePattern"));
        assert!(json.contains("elements"));
    }

    #[test]
    fn test_pattern_literal_v4_serialization() {
        use crate::ir::ValueAttributes;
        let pat: Pattern<ValueAttributes> =
            Pattern::LiteralPattern(ValueAttributes::default(), Literal::Integer(42));
        let json = serde_json::to_string(&PatternWrapper(&pat)).unwrap();
        assert!(json.contains("LiteralPattern"));
        assert!(json.contains("IntegerLiteral"));
        assert!(json.contains("42"));
    }

    #[test]
    fn test_pattern_unit_v4_serialization() {
        use crate::ir::ValueAttributes;
        let pat: Pattern<ValueAttributes> = Pattern::UnitPattern(ValueAttributes::default());
        let json = serde_json::to_string(&PatternWrapper(&pat)).unwrap();
        assert!(json.contains("UnitPattern"));
    }

    // Value tests
    struct ValueWrapper<'a, TA: Clone + Serialize, VA: Clone + Serialize>(&'a Value<TA, VA>);

    impl<'a, TA: Clone + Serialize, VA: Clone + Serialize> Serialize for ValueWrapper<'a, TA, VA> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serialize_value(self.0, serializer)
        }
    }

    #[test]
    fn test_value_unit_v4_serialization() {
        use crate::ir::ValueAttributes;
        let val: Value<TypeAttributes, ValueAttributes> = Value::Unit(ValueAttributes::default());
        let json = serde_json::to_string(&ValueWrapper(&val)).unwrap();
        assert!(json.contains("Unit"));
    }

    #[test]
    fn test_value_literal_v4_serialization() {
        use crate::ir::ValueAttributes;
        let val: Value<TypeAttributes, ValueAttributes> =
            Value::Literal(ValueAttributes::default(), Literal::Integer(42));
        let json = serde_json::to_string(&ValueWrapper(&val)).unwrap();
        assert!(json.contains("Literal"));
        assert!(json.contains("IntegerLiteral"));
        assert!(json.contains("42"));
    }

    #[test]
    fn test_value_variable_v4_serialization() {
        use crate::ir::ValueAttributes;
        let val: Value<TypeAttributes, ValueAttributes> =
            Value::Variable(ValueAttributes::default(), Name::from("x"));
        let json = serde_json::to_string(&ValueWrapper(&val)).unwrap();
        assert!(json.contains("Variable"));
        assert!(json.contains("\"x\""));
    }

    #[test]
    fn test_value_tuple_v4_serialization() {
        use crate::ir::ValueAttributes;
        let val: Value<TypeAttributes, ValueAttributes> = Value::Tuple(
            ValueAttributes::default(),
            vec![
                Value::Unit(ValueAttributes::default()),
                Value::Unit(ValueAttributes::default()),
            ],
        );
        let json = serde_json::to_string(&ValueWrapper(&val)).unwrap();
        assert!(json.contains("Tuple"));
        assert!(json.contains("elements"));
    }

    #[test]
    fn test_value_apply_v4_serialization() {
        use crate::ir::ValueAttributes;
        let val: Value<TypeAttributes, ValueAttributes> = Value::Apply(
            ValueAttributes::default(),
            Box::new(Value::Variable(ValueAttributes::default(), Name::from("f"))),
            Box::new(Value::Variable(ValueAttributes::default(), Name::from("x"))),
        );
        let json = serde_json::to_string(&ValueWrapper(&val)).unwrap();
        assert!(json.contains("Apply"));
        assert!(json.contains("function"));
        assert!(json.contains("argument"));
    }

    #[test]
    fn test_value_lambda_v4_serialization() {
        use crate::ir::ValueAttributes;
        let val: Value<TypeAttributes, ValueAttributes> = Value::Lambda(
            ValueAttributes::default(),
            Pattern::WildcardPattern(ValueAttributes::default()),
            Box::new(Value::Unit(ValueAttributes::default())),
        );
        let json = serde_json::to_string(&ValueWrapper(&val)).unwrap();
        assert!(json.contains("Lambda"));
        assert!(json.contains("argumentPattern"));
        assert!(json.contains("body"));
    }
}

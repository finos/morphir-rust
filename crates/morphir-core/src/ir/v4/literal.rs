//! Literal values for Morphir IR.
//!
//! This module defines the `Literal` type which represents constant values
//! that can appear in Morphir IR expressions.
//!
//! Serialization uses V4 object wrapper format:
//! - `{ "IntegerLiteral": { "value": 42 } }`
//! - `{ "StringLiteral": { "value": "hello" } }`
//!
//! Deserialization accepts V4 and Classic formats for backward compatibility.

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Literal constant values.
///
/// Represents the basic literal types supported by Morphir IR.
/// These are values that can be embedded directly in the IR without
/// any runtime computation.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    /// Boolean literal (true or false)
    Bool(bool),

    /// Character literal (single Unicode character)
    Char(char),

    /// String literal (UTF-8 text)
    String(String),

    /// Integer literal
    Integer(i64),

    /// Floating-point literal
    Float(f64),

    /// Decimal literal (stored as string for arbitrary precision)
    Decimal(String),
}

// V4 serialization: { "IntegerLiteral": { "value": 42 } }
impl Serialize for Literal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct LiteralValue<T: Serialize> {
            value: T,
        }

        match self {
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
}

// V4 deserialization with Classic fallback
impl<'de> Deserialize<'de> for Literal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(LiteralVisitor)
    }
}

struct LiteralVisitor;

impl<'de> Visitor<'de> for LiteralVisitor {
    type Value = Literal;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(
            "V4 object { \"IntegerLiteral\": { \"value\": 42 } } or Classic array",
        )
    }

    /// V4 object wrapper format: { "IntegerLiteral": { "value": 42 } }
    fn visit_map<M>(self, mut map: M) -> Result<Literal, M::Error>
    where
        M: MapAccess<'de>,
    {
        #[derive(Deserialize)]
        struct LiteralValue<T> {
            value: T,
        }

        let (tag, value): (String, serde_json::Value) = map
            .next_entry()?
            .ok_or_else(|| de::Error::custom("expected object wrapper with single key"))?;

        match tag.as_str() {
            "BoolLiteral" => {
                let content: LiteralValue<bool> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(Literal::Bool(content.value))
            }
            "CharLiteral" => {
                let content: LiteralValue<String> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let c = content
                    .value
                    .chars()
                    .next()
                    .ok_or_else(|| de::Error::custom("empty char literal"))?;
                Ok(Literal::Char(c))
            }
            "StringLiteral" => {
                let content: LiteralValue<String> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(Literal::String(content.value))
            }
            "IntegerLiteral" | "WholeNumberLiteral" => {
                let content: LiteralValue<i64> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(Literal::Integer(content.value))
            }
            "FloatLiteral" => {
                let content: LiteralValue<f64> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(Literal::Float(content.value))
            }
            "DecimalLiteral" => {
                let content: LiteralValue<String> =
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

    /// Classic tagged array format: ["IntegerLiteral", 42]
    fn visit_seq<V>(self, mut seq: V) -> Result<Literal, V::Error>
    where
        V: SeqAccess<'de>,
    {
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

impl Literal {
    /// Create a new boolean literal
    pub fn bool(value: bool) -> Self {
        Literal::Bool(value)
    }

    /// Create a new character literal
    pub fn char(value: char) -> Self {
        Literal::Char(value)
    }

    /// Create a new string literal
    pub fn string(value: impl Into<String>) -> Self {
        Literal::String(value.into())
    }

    /// Create a new integer literal
    pub fn integer(value: i64) -> Self {
        Literal::Integer(value)
    }

    /// Create a new float literal
    pub fn float(value: f64) -> Self {
        Literal::Float(value)
    }

    /// Create a new decimal literal from a string representation
    pub fn decimal(value: impl Into<String>) -> Self {
        Literal::Decimal(value.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_constructors() {
        assert_eq!(Literal::bool(true), Literal::Bool(true));
        assert_eq!(Literal::char('a'), Literal::Char('a'));
        assert_eq!(
            Literal::string("hello"),
            Literal::String("hello".to_string())
        );
        assert_eq!(Literal::integer(42), Literal::Integer(42));
        assert_eq!(Literal::float(2.5), Literal::Float(2.5));
        assert_eq!(
            Literal::decimal("123.456"),
            Literal::Decimal("123.456".to_string())
        );
    }

    #[test]
    fn test_literal_clone() {
        let lit = Literal::String("test".to_string());
        let cloned = lit.clone();
        assert_eq!(lit, cloned);
    }
}

//! Classic IR Literal types
//!
//! Literal values for the Classic Morphir IR format.

use serde::de::{self, IgnoredAny, SeqAccess, Visitor};
use serde::ser::{SerializeTuple, Serializer};
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::fmt;

/// Literal values - serialized as ["LiteralType", value]
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Bool(bool),
    Char(char),
    String(String),
    WholeNumber(i64),
    Float(f64),
}

impl Serialize for Literal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Literal::Bool(v) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("BoolLiteral")?;
                tuple.serialize_element(v)?;
                tuple.end()
            }
            Literal::Char(v) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("CharLiteral")?;
                tuple.serialize_element(v)?;
                tuple.end()
            }
            Literal::String(v) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("StringLiteral")?;
                tuple.serialize_element(v)?;
                tuple.end()
            }
            Literal::WholeNumber(v) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("WholeNumberLiteral")?;
                tuple.serialize_element(v)?;
                tuple.end()
            }
            Literal::Float(v) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("FloatLiteral")?;
                tuple.serialize_element(v)?;
                tuple.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Literal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LiteralVisitor;

        impl<'de> Visitor<'de> for LiteralVisitor {
            type Value = Literal;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a Literal array [\"LiteralType\", value]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let tag: Cow<'de, str> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;

                match tag.as_ref() {
                    "BoolLiteral" | "bool_literal" => {
                        let v = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                        if seq.next_element::<IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom("Expected end of BoolLiteral array"));
                        }

                        Ok(Literal::Bool(v))
                    }
                    "CharLiteral" | "char_literal" => {
                        let v = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                        if seq.next_element::<IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom("Expected end of CharLiteral array"));
                        }

                        Ok(Literal::Char(v))
                    }
                    "StringLiteral" | "string_literal" => {
                        let v = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                        if seq.next_element::<IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom("Expected end of StringLiteral array"));
                        }

                        Ok(Literal::String(v))
                    }
                    "WholeNumberLiteral"
                    | "whole_number_literal"
                    | "IntLiteral"
                    | "int_literal" => {
                        let v = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                        if seq.next_element::<IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom("Expected end of IntLiteral array"));
                        }

                        Ok(Literal::WholeNumber(v))
                    }
                    "FloatLiteral" | "float_literal" => {
                        let v = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                        if seq.next_element::<IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom("Expected end of FloatLiteral array"));
                        }

                        Ok(Literal::Float(v))
                    }
                    "DecimalLiteral" | "decimal_literal" => {
                        // Decimal is represented as a string in JSON, store as Float for now
                        let v: String = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let f = v.parse::<f64>().map_err(de::Error::custom)?;
                        if seq.next_element::<IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom("Expected end of DecimalLiteral array"));
                        }
                        Ok(Literal::Float(f))
                    }
                    _ => Err(de::Error::unknown_variant(
                        tag.as_ref(),
                        &[
                            "BoolLiteral",
                            "CharLiteral",
                            "StringLiteral",
                            "WholeNumberLiteral",
                            "FloatLiteral",
                            "DecimalLiteral",
                        ],
                    )),
                }
            }
        }

        deserializer.deserialize_seq(LiteralVisitor)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_literal_bool() {
        let l = Literal::Bool(true);
        let json = serde_json::to_string(&l).unwrap();
        assert_eq!(json, r#"["BoolLiteral",true]"#);
        let deserialized: Literal = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, l);
    }

    #[test]
    fn test_serialize_literal_char() {
        let l = Literal::Char('a');
        let json = serde_json::to_string(&l).unwrap();
        assert_eq!(json, r#"["CharLiteral","a"]"#);
        let deserialized: Literal = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, l);
    }

    #[test]
    fn test_serialize_literal_string() {
        let l = Literal::String("hello".to_string());
        let json = serde_json::to_string(&l).unwrap();
        assert_eq!(json, r#"["StringLiteral","hello"]"#);
        let deserialized: Literal = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, l);
    }

    #[test]
    fn test_serialize_literal_whole_number() {
        let l = Literal::WholeNumber(123);
        let json = serde_json::to_string(&l).unwrap();
        assert_eq!(json, r#"["WholeNumberLiteral",123]"#);
        let deserialized: Literal = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, l);
    }

    #[test]
    fn test_serialize_literal_float() {
        let l = Literal::Float(1.23);
        let json = serde_json::to_string(&l).unwrap();
        assert_eq!(json, r#"["FloatLiteral",1.23]"#);
        let deserialized: Literal = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, l);
    }

    #[test]
    fn test_deserialize_literal_decimal() {
        let json = r#"["DecimalLiteral","1.23"]"#;
        let deserialized: Literal = serde_json::from_str(json).unwrap();
        match deserialized {
            Literal::Float(f) => assert!((f - 1.23).abs() < f64::EPSILON),
            _ => panic!("Expected Float from DecimalLiteral"),
        }
    }
}

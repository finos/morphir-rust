//! Classic IR Literal types
//!
//! Literal values for the Classic Morphir IR format.

use serde::de::{self, SeqAccess, Visitor};
use serde::ser::{SerializeTuple, Serializer};
use serde::{Deserialize, Deserializer, Serialize};
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
                let tag: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;

                match tag.as_str() {
                    "BoolLiteral" | "bool_literal" => {
                        let v = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        
                        if let Some(_) = seq.next_element::<serde_json::Value>()? {
                             return Err(de::Error::custom("Expected end of BoolLiteral array"));
                        }

                        Ok(Literal::Bool(v))
                    }
                    "CharLiteral" | "char_literal" => {
                        let v = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                        if let Some(_) = seq.next_element::<serde_json::Value>()? {
                             return Err(de::Error::custom("Expected end of CharLiteral array"));
                        }

                        Ok(Literal::Char(v))
                    }
                    "StringLiteral" | "string_literal" => {
                        let v = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                        if let Some(_) = seq.next_element::<serde_json::Value>()? {
                             return Err(de::Error::custom("Expected end of StringLiteral array"));
                        }

                        Ok(Literal::String(v))
                    }
                    "WholeNumberLiteral" | "whole_number_literal" | "IntLiteral" | "int_literal" => {
                        let v = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        
                        if let Some(_) = seq.next_element::<serde_json::Value>()? {
                             return Err(de::Error::custom("Expected end of IntLiteral array"));
                        }
                        
                        Ok(Literal::WholeNumber(v))
                    }
                    "FloatLiteral" | "float_literal" => {
                        let v = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                        if let Some(_) = seq.next_element::<serde_json::Value>()? {
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
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of DecimalLiteral array")); }
                        Ok(Literal::Float(f))
                    }
                    _ => Err(de::Error::unknown_variant(
                        &tag,
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

//! Literal values for Morphir IR.
//!
//! This module defines the `Literal` type which represents constant values
//! that can appear in Morphir IR expressions.
//!
//! A literal is a single-member wrapper whose payload is the value itself:
//! `{ "IntegerLiteral": 42 }`, `{ "CharLiteral": "a" }`, `{ "DecimalLiteral": "10.50" }`.
//! `{ "IntegerLiteral": { "value": 42 } }` is also accepted, but never written.
//!
//! The decode lives in [`super::serde_tagged`] and the encode in [`super::serde_v4`], so every
//! v4 node reports its diagnostics the same way.

use serde::{Serialize, Serializer};

use super::serde_v4;

/// Literal constant values.
///
/// Represents the basic literal types supported by Morphir IR.
/// These are values that can be embedded directly in the IR without
/// any runtime computation.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    /// Boolean literal (true or false): `{ "BoolLiteral": true }`
    Bool(bool),

    /// Character literal, one code point carried as a string: `{ "CharLiteral": "a" }`
    Char(char),

    /// String literal (UTF-8 text): `{ "StringLiteral": "s" }`
    String(String),

    /// Integer literal: `{ "IntegerLiteral": 42 }`
    Integer(i64),

    /// Floating-point literal: `{ "FloatLiteral": 1.5 }`
    Float(f64),

    /// Decimal literal, carried as text so no binding coerces it to a float:
    /// `{ "DecimalLiteral": "10.50" }`
    Decimal(String),

    /// Document literal: a schema-less JSON-like tree carried verbatim, typed as
    /// `morphir/SDK:document#document`. Its payload is the document itself, so
    /// `{ "DocumentLiteral": { "value": 1 } }` is the one-member document `{ "value": 1 }`.
    ///
    /// Number lexemes survive the round trip, which is why `serde_json` is built here with
    /// `arbitrary_precision`. See the decision on the document literal on
    /// <https://morphir.finos.org/docs/spec/ir/>.
    Document(serde_json::Value),
}

impl Serialize for Literal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serde_v4::serialize_literal(self, serializer)
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

    /// Create a new document literal from a JSON-like tree
    pub fn document(value: serde_json::Value) -> Self {
        Literal::Document(value)
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
        assert_eq!(
            Literal::document(serde_json::json!({ "a": 1 })),
            Literal::Document(serde_json::json!({ "a": 1 }))
        );
    }

    #[test]
    fn test_literal_clone() {
        let lit = Literal::String("test".to_string());
        let cloned = lit.clone();
        assert_eq!(lit, cloned);
    }
}

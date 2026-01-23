//! Literal values for Morphir IR.
//!
//! This module defines the `Literal` type which represents constant values
//! that can appear in Morphir IR expressions.

use serde::{Deserialize, Serialize};

/// Literal constant values.
///
/// Represents the basic literal types supported by Morphir IR.
/// These are values that can be embedded directly in the IR without
/// any runtime computation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "0", content = "1")]
pub enum Literal {
    /// Boolean literal (true or false)
    #[serde(rename = "BoolLiteral")]
    Bool(bool),

    /// Character literal (single Unicode character)
    #[serde(rename = "CharLiteral")]
    Char(char),

    /// String literal (UTF-8 text)
    #[serde(rename = "StringLiteral")]
    String(String),

    /// Integer literal (V4 name, accepts WholeNumberLiteral on deserialize)
    #[serde(rename = "IntegerLiteral", alias = "WholeNumberLiteral")]
    Integer(i64),

    /// Floating-point literal
    #[serde(rename = "FloatLiteral")]
    Float(f64),

    /// Decimal literal (stored as string for arbitrary precision)
    #[serde(rename = "DecimalLiteral")]
    Decimal(String),
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

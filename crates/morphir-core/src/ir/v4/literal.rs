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

use std::str::FromStr;

use serde::{Serialize, Serializer};

use super::serde_v4;

/// A lexeme that is not the spelling of a finite JSON number, and so is not a float literal.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{lexeme:?} is not the spelling of a finite JSON number")]
pub struct InvalidFloatLexeme {
    lexeme: String,
}

impl InvalidFloatLexeme {
    fn new(lexeme: &str) -> Self {
        Self {
            lexeme: lexeme.to_owned(),
        }
    }

    /// The text that was offered as a float literal.
    pub fn lexeme(&self) -> &str {
        &self.lexeme
    }
}

/// A floating-point literal and the text it was written with.
///
/// The value is the machine number the model computes with; the lexeme is the spelling the
/// source carried. A writer that must reproduce its input byte for byte — the YAML profile, for
/// one — writes the lexeme, so `1.0e2` does not come back out as `100.0`.
///
/// The [`serde_json::Number`] the lexeme was validated as is carried alongside it, so a
/// serializer writes the number the literal was built from rather than re-parsing text that might
/// not be a number: every `FloatLiteral` that exists is a finite JSON number by construction. Its
/// spelling is serde_json's own — an exponent is written `e+2` whatever the lexeme wrote — which
/// is why the lexeme is kept as well.
///
/// Two float literals are the same literal when they denote the same number, so the lexeme is
/// carried but is not part of identity.
#[derive(Debug, Clone)]
pub struct FloatLiteral {
    value: f64,
    lexeme: String,
    number: serde_json::Number,
}

impl FloatLiteral {
    /// Read a float literal from the text it was written with, keeping that text verbatim.
    ///
    /// The text has to be a JSON number — so `.5`, `5.`, `+1.5`, `NaN` and `inf` are not float
    /// literals however readily Rust's own `f64` parser takes them — and it has to denote a
    /// finite value, which rules out a magnitude no `f64` can hold, such as `1e400`.
    pub fn from_lexeme(lexeme: &str) -> Result<Self, InvalidFloatLexeme> {
        let number =
            serde_json::Number::from_str(lexeme).map_err(|_| InvalidFloatLexeme::new(lexeme))?;
        let value = number
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(|| InvalidFloatLexeme::new(lexeme))?;
        Ok(Self {
            value,
            lexeme: lexeme.to_owned(),
            number,
        })
    }

    /// Build a float literal from a machine number, spelling it the shortest way that reads back
    /// as the same number.
    ///
    /// Rust's `Debug` for `f64` is that shortest round-trip form and always carries a `.0` or an
    /// exponent, so the result is a JSON number that reads back as a float rather than an
    /// integer.
    ///
    /// # Panics
    ///
    /// Panics when `value` is not finite: an infinity or a NaN has no JSON spelling, and a model
    /// value is never one.
    pub fn from_f64(value: f64) -> Self {
        assert!(
            value.is_finite(),
            "a FloatLiteral holds a finite number, not {value:?}"
        );
        // `{value:?}` of a finite `f64` always carries a `.0` or an exponent, so it is a JSON
        // number and `from_lexeme` accepts it.
        Self::from_lexeme(&format!("{value:?}")).expect("a finite f64 spells a JSON number")
    }

    /// The number this literal denotes.
    pub fn value(&self) -> f64 {
        self.value
    }

    /// The text this literal was written with, verbatim.
    pub fn lexeme(&self) -> &str {
        &self.lexeme
    }

    /// This literal as the JSON number it was validated as.
    pub fn number(&self) -> &serde_json::Number {
        &self.number
    }
}

/// A float literal is identified by the number it denotes, not by how it was spelled.
impl PartialEq for FloatLiteral {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

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
    ///
    /// The literal carries the text it was written with alongside its value, so a writer that
    /// reproduces its input — the YAML profile — spells `1.0e2` the way it was read.
    Float(FloatLiteral),

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
        Literal::Float(FloatLiteral::from_f64(value))
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
        assert_eq!(
            Literal::float(2.5),
            Literal::Float(FloatLiteral::from_f64(2.5))
        );
        assert_eq!(
            Literal::decimal("123.456"),
            Literal::Decimal("123.456".to_string())
        );
        assert_eq!(
            Literal::document(serde_json::json!({ "a": 1 })),
            Literal::Document(serde_json::json!({ "a": 1 }))
        );
    }

    /// A lexeme Rust's own `f64` parser takes but JSON does not is not a float literal: the
    /// spelling a `FloatLiteral` carries is written back out as a JSON number, so anything that
    /// is not one would be unwritable.
    #[test]
    fn a_lexeme_that_is_not_a_json_number_is_refused() {
        for lexeme in [".5", "5.", "+1.5", "NaN", "nan", "inf", "-inf", "Infinity"] {
            assert_eq!(
                FloatLiteral::from_lexeme(lexeme)
                    .expect_err(lexeme)
                    .lexeme(),
                lexeme
            );
        }
    }

    /// A JSON number too large for an `f64` denotes no machine value, so it is refused with the
    /// same error rather than becoming an infinity.
    #[test]
    fn a_lexeme_outside_the_f64_range_is_refused() {
        assert!(FloatLiteral::from_lexeme("1e400").is_err());
        assert!(FloatLiteral::from_lexeme("-1e400").is_err());
    }

    /// The spellings the profile does carry survive verbatim.
    #[test]
    fn a_json_number_keeps_its_spelling() {
        for lexeme in ["4", "4.0", "1.0e2", "1.50", "-0.0", "0e0"] {
            assert_eq!(
                FloatLiteral::from_lexeme(lexeme).expect(lexeme).lexeme(),
                lexeme
            );
        }
    }

    #[test]
    fn test_literal_clone() {
        let lit = Literal::String("test".to_string());
        let cloned = lit.clone();
        assert_eq!(lit, cloned);
    }
}

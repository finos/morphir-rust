//! A decimal literal is a genuine decimal.
//!
//! The value is a [`BigDecimal`]; the lexeme is the text the literal was written with, which is
//! what the wire carries (a JSON string) and what a writer emits. `bigdecimal` normalises what it
//! parses, so the lexeme is the only record of the written scale and spelling: `10.50` and `10.5`
//! denote the same number and are different literals, which is Java's `BigDecimal.equals` rule
//! and the rule the kit's byte-wise canonical comparison needs.
//!
//! The grammar is the one the v4 schema page states under "Literals"; both Rust models and the
//! reference apply it before any parser sees the text, so what is refused does not depend on a
//! library's leniency.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use bigdecimal::BigDecimal;

/// A string that is not a decimal lexeme.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{lexeme:?} is not a decimal lexeme")]
pub struct InvalidDecimalLexeme {
    lexeme: String,
}

impl InvalidDecimalLexeme {
    /// The text that was offered as a decimal literal.
    pub fn lexeme(&self) -> &str {
        &self.lexeme
    }
}

/// Whether `text` is `[+-]? ( digits ( "." digits? )? | "." digits ) ( [eE] [+-]? digits )?`.
pub fn is_decimal_lexeme(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        i += 1;
    }
    let integer_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let integer_digits = i - integer_start;
    let mut fraction_digits = 0;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let fraction_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        fraction_digits = i - fraction_start;
    }
    if integer_digits == 0 && fraction_digits == 0 {
        return false;
    }
    if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
        i += 1;
        if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
            i += 1;
        }
        let exponent_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == exponent_start {
            return false;
        }
    }
    i == bytes.len()
}

/// A decimal literal: the number it denotes and the text it was written with.
#[derive(Debug, Clone)]
pub struct DecimalLiteral {
    value: BigDecimal,
    lexeme: String,
}

impl DecimalLiteral {
    /// Read a decimal literal from its lexeme, keeping the lexeme verbatim.
    pub fn parse(lexeme: &str) -> Result<Self, InvalidDecimalLexeme> {
        let invalid = || InvalidDecimalLexeme {
            lexeme: lexeme.to_owned(),
        };
        if !is_decimal_lexeme(lexeme) {
            return Err(invalid());
        }
        // The grammar admits `12.` and `.5`; `bigdecimal`'s parser wants a digit on each side of
        // the point, so the text handed to it is completed with a zero. The lexeme is untouched.
        let mut completed = lexeme.to_owned();
        let sign_len = usize::from(completed.starts_with(['+', '-']));
        if completed[sign_len..].starts_with('.') {
            completed.insert(sign_len, '0');
        }
        if let Some(point) = completed.find('.')
            && !completed[point + 1..].starts_with(|c: char| c.is_ascii_digit())
        {
            completed.insert(point + 1, '0');
        }
        let value = BigDecimal::from_str(&completed).map_err(|_| invalid())?;
        Ok(Self {
            value,
            lexeme: lexeme.to_owned(),
        })
    }

    /// Build a literal from a value, spelling it in plain positional notation.
    pub fn from_value(value: BigDecimal) -> Self {
        let lexeme = value.to_plain_string();
        Self { value, lexeme }
    }

    /// The number this literal denotes.
    pub fn value(&self) -> &BigDecimal {
        &self.value
    }

    /// The text this literal was written with, verbatim.
    pub fn lexeme(&self) -> &str {
        &self.lexeme
    }
}

/// Two decimal literals are the same literal when they were written the same way.
impl PartialEq for DecimalLiteral {
    fn eq(&self, other: &Self) -> bool {
        self.lexeme == other.lexeme
    }
}

impl Eq for DecimalLiteral {}

impl Hash for DecimalLiteral {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.lexeme.hash(state);
    }
}

impl fmt::Display for DecimalLiteral {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.lexeme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn the_grammar_accepts_what_the_schema_page_names() {
        for lexeme in [
            "0", "10.50", "-0.00", "+12.", ".5", "1e-7", "1E+3", "-.5e2", "007",
        ] {
            assert!(is_decimal_lexeme(lexeme), "{lexeme}");
        }
    }

    #[test]
    fn the_grammar_refuses_everything_else() {
        for lexeme in [
            "",
            "ten",
            ".",
            "+",
            "1_000",
            "0x10",
            "NaN",
            "Infinity",
            "-Infinity",
            "1e",
            "1e+",
            " 1",
            "1 ",
            "1.5.2",
            "1,5",
        ] {
            assert!(!is_decimal_lexeme(lexeme), "{lexeme}");
        }
    }

    #[test]
    fn a_literal_keeps_its_lexeme_and_carries_the_value() {
        let literal = DecimalLiteral::parse("10.50").unwrap();
        assert_eq!(literal.lexeme(), "10.50");
        assert_eq!(
            literal.value(),
            &bigdecimal::BigDecimal::from_str("10.5").unwrap()
        );
        for lexeme in ["+12.", ".5", "-.5e2", "1e-7"] {
            let parsed = DecimalLiteral::parse(lexeme).expect(lexeme);
            assert_eq!(parsed.lexeme(), lexeme);
        }
        assert_eq!(DecimalLiteral::parse("ten").unwrap_err().lexeme(), "ten");
    }

    #[test]
    fn identity_is_the_lexeme_not_the_number() {
        let scaled = DecimalLiteral::parse("10.50").unwrap();
        let plain = DecimalLiteral::parse("10.5").unwrap();
        assert_ne!(scaled, plain);
        assert_eq!(scaled.value(), plain.value());
        assert_eq!(scaled, DecimalLiteral::parse("10.50").unwrap());
    }

    #[test]
    fn from_value_spells_the_plain_form() {
        let value = bigdecimal::BigDecimal::from_str("1.50").unwrap();
        assert_eq!(DecimalLiteral::from_value(value).lexeme(), "1.50");
    }
}

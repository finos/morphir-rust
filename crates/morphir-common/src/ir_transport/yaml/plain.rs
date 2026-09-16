//! A YAML-writable image of a value, free of serde_json's private number token.

use morphir_core::traversal::IrCursor;
use serde::Serialize;
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde_json::Value as JsonValue;

use super::YamlCodec;
use crate::ir_transport::{Stage, TransportDiagnostic};

/// A value rewritten so every part of it is something serde-saphyr can spell.
///
/// `morphir-core` builds serde_json with `arbitrary_precision` so a `DocumentLiteral` carries a
/// number's lexeme verbatim, and cargo unifies that feature across the workspace. The feature
/// makes a `serde_json::Number` serialize as a private one-field struct named
/// `$serde_json::private::Number`, which only serde_json's own serializer recognizes; handed
/// straight to serde-saphyr, a document literal holding a number would be written into the YAML
/// as a mapping under that private name. Going through serde_json's own value first is what
/// resolves the token — that is the one serializer that reads it — and each number is then read
/// back here as the plain number YAML writes.
///
/// Every other node is carried across unchanged and serializes exactly as the value it was built
/// from does: serde-saphyr writes a struct as a mapping, a unit and a `None` alike as `null`, and
/// a char as a string.
///
/// # Numbers serde-saphyr cannot spell
///
/// A lexeme is only carried across if one of the number types below writes it back as the same
/// number. serde-saphyr has no way to emit a scalar verbatim — a serializer reaches it through
/// serde's data model, whose widest number is `u64`, `i64`, or `f64`, and every wrapper it offers
/// (`FlowSeq`, `DoubleQuoted`, `Tagged`, and the rest) decorates one of those or a string, none of
/// which writes a bare number from a lexeme. So a `DocumentLiteral` carrying, say,
/// `0.123456789012345678901` is refused by [`PlainValue::of`] rather than written rounded or
/// retyped as a string: an artifact that reads back as a different number is worse than one that
/// was never written. See the changelog's Known limitations.
pub(super) enum PlainValue {
    Null,
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    Text(String),
    Seq(Vec<PlainValue>),
    Map(Vec<(String, PlainValue)>),
}

impl PlainValue {
    /// The YAML-writable image of `value`.
    pub(super) fn of<T: Serialize + ?Sized>(value: &T) -> Result<Self, TransportDiagnostic> {
        let json = serde_json::to_value(value).map_err(YamlCodec::encode_error)?;
        Self::from_json(&json)
    }

    fn from_json(value: &JsonValue) -> Result<Self, TransportDiagnostic> {
        Ok(match value {
            JsonValue::Null => PlainValue::Null,
            JsonValue::Bool(value) => PlainValue::Bool(*value),
            JsonValue::Number(number) => {
                // The widest reading of the lexeme that keeps every digit wins. An integer is
                // exact in `u64` or `i64` by construction; a float is only taken when what it
                // writes back is the same number the lexeme spells.
                if let Some(value) = number.as_u64() {
                    PlainValue::Unsigned(value)
                } else if let Some(value) = number.as_i64() {
                    PlainValue::Signed(value)
                } else {
                    let lexeme = number.to_string();
                    match number.as_f64() {
                        Some(value) if writes_back_as(value, &lexeme) => PlainValue::Float(value),
                        _ => return Err(unwritable_number(&lexeme)),
                    }
                }
            }
            JsonValue::String(value) => PlainValue::Text(value.clone()),
            JsonValue::Array(members) => PlainValue::Seq(
                members
                    .iter()
                    .map(PlainValue::from_json)
                    .collect::<Result<_, _>>()?,
            ),
            JsonValue::Object(members) => PlainValue::Map(
                members
                    .iter()
                    .map(|(name, member)| Ok((name.clone(), PlainValue::from_json(member)?)))
                    .collect::<Result<_, TransportDiagnostic>>()?,
            ),
        })
    }
}

/// Whether writing `value` as a YAML number spells the same number `lexeme` does.
///
/// Not whether the `f64` *is* the lexeme's value: `0.1` is no more exactly representable in binary
/// than any other tenth, and refusing it would refuse nearly every decimal. What matters is the
/// text that comes out. serde-saphyr writes an `f64` as its shortest round-tripping decimal, which
/// is what `Display` produces too, so the question is whether that decimal and the lexeme are the
/// same number — trailing zeros and exponent form aside, which [`canonical_decimal`] sets aside.
fn writes_back_as(value: f64, lexeme: &str) -> bool {
    if !value.is_finite() {
        return false;
    }
    match (
        canonical_decimal(lexeme),
        canonical_decimal(&value.to_string()),
    ) {
        (Some(from_lexeme), Some(written)) => from_lexeme == written,
        _ => false,
    }
}

/// A decimal as its sign, its significant digits, and the power of ten they sit at, so two
/// spellings of one number — `0.10` and `0.1`, `1e3` and `1000` — compare equal.
///
/// `None` if `text` is not a decimal number, which for a JSON lexeme or a finite `f64`'s `Display`
/// cannot happen; a caller reads it as "these are not the same number".
fn canonical_decimal(text: &str) -> Option<(bool, String, i64)> {
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (mantissa, exponent) = match rest.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i64>().ok()?),
        None => (rest, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    if !whole
        .bytes()
        .chain(fraction.bytes())
        .all(|b| b.is_ascii_digit())
    {
        return None;
    }

    let mut digits = format!("{whole}{fraction}");
    let mut exponent = exponent.checked_sub(i64::try_from(fraction.len()).ok()?)?;
    let leading = digits.len() - digits.trim_start_matches('0').len();
    digits.drain(..leading);
    while digits.ends_with('0') {
        digits.pop();
        exponent = exponent.checked_add(1)?;
    }
    if digits.is_empty() {
        // Zero has one spelling here, whatever its sign or exponent was.
        return Some((false, "0".to_string(), 0));
    }
    Some((negative, digits, exponent))
}

/// The refusal a number no YAML scalar this encoder can write would carry exactly.
fn unwritable_number(lexeme: &str) -> TransportDiagnostic {
    TransportDiagnostic::error(
        "morphir::ir::yaml::invalid_literal",
        Stage::Encoding,
        IrCursor::root(),
        format!(
            "the number {lexeme} needs more precision than the YAML encoder can carry exactly; \
             writing it would change its value"
        ),
    )
    .with_guidance("encode this document as JSON, or spell the number within f64's precision")
}

impl Serialize for PlainValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            PlainValue::Null => serializer.serialize_unit(),
            PlainValue::Bool(value) => serializer.serialize_bool(*value),
            PlainValue::Unsigned(value) => serializer.serialize_u64(*value),
            PlainValue::Signed(value) => serializer.serialize_i64(*value),
            PlainValue::Float(value) => serializer.serialize_f64(*value),
            PlainValue::Text(value) => serializer.serialize_str(value),
            PlainValue::Seq(members) => {
                let mut sequence = serializer.serialize_seq(Some(members.len()))?;
                for member in members {
                    sequence.serialize_element(member)?;
                }
                sequence.end()
            }
            PlainValue::Map(members) => {
                let mut map = serializer.serialize_map(Some(members.len()))?;
                for (name, member) in members {
                    map.serialize_entry(name, member)?;
                }
                map.end()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use morphir_core::ir::v4::{Literal, Value, ValueAttributes};

    use super::super::profile::to_yaml_text;
    use crate::ir_transport::Stage;

    /// A value whose document literal is `payload`, parsed from text so the lexeme survives.
    fn document(payload: &str) -> Value {
        let payload = serde_json::from_str(payload).unwrap();
        Value::Literal(ValueAttributes::default(), Literal::Document(payload))
    }

    #[test]
    fn a_document_literal_writes_its_numbers_as_yaml_numbers() {
        // Parsed from text: only the text carries the lexeme a host's own JSON value would
        // already have rounded, which is the whole reason `arbitrary_precision` is on.
        let payload = serde_json::from_str(r#"{ "id": 9007199254740993, "ratio": 0.10 }"#).unwrap();
        let value = Value::Literal(ValueAttributes::default(), Literal::Document(payload));

        let yaml = to_yaml_text(&value).unwrap();

        assert!(
            !yaml.contains("serde_json"),
            "serde_json's private number token leaked into the YAML: {yaml}"
        );
        assert!(yaml.contains("9007199254740993"), "{yaml}");
        assert!(yaml.contains("0.1"), "{yaml}");
    }

    #[test]
    fn an_in_range_number_stays_a_bare_scalar() {
        // Neither an integer nor a tenth is exactly a binary float, and neither needs to be: what
        // the encoder writes is the decimal, and it is the one the lexeme spells.
        let yaml = to_yaml_text(&document(
            r#"{ "ratio": 0.10, "scale": 1e3, "tiny": -2.5e-7 }"#,
        ))
        .unwrap();

        assert!(yaml.contains("ratio: 0.1"), "{yaml}");
        assert!(yaml.contains("scale: 1000"), "{yaml}");
        assert!(yaml.contains("tiny: -2.5e-7"), "{yaml}");
        assert!(!yaml.contains('"') && !yaml.contains('\''), "{yaml}");
    }

    #[test]
    fn a_number_too_precise_for_the_encoder_is_refused() {
        for lexeme in [
            // More digits than an f64 keeps.
            "0.123456789012345678901",
            // An integer wider than u64 and not a round one, so the nearest f64 writes back as a
            // different number.
            "123456789012345678901234567890",
        ] {
            let diagnostic =
                to_yaml_text(&document(&format!("{{ \"n\": {lexeme} }}"))).expect_err(lexeme);

            assert_eq!(diagnostic.code(), "morphir::ir::yaml::invalid_literal");
            assert_eq!(diagnostic.stage(), Stage::Encoding);
            assert!(diagnostic.message().contains(lexeme), "{diagnostic:?}");
        }
    }

    #[test]
    fn canonical_decimal_sets_aside_spelling_but_not_value() {
        use super::canonical_decimal;

        assert_eq!(canonical_decimal("0.10"), canonical_decimal("1e-1"));
        assert_eq!(canonical_decimal("1000"), canonical_decimal("1.0e3"));
        assert_eq!(canonical_decimal("-0"), canonical_decimal("0.000"));
        assert_ne!(canonical_decimal("0.1"), canonical_decimal("0.10000000001"));
        assert_ne!(canonical_decimal("1"), canonical_decimal("-1"));
        assert_eq!(canonical_decimal("what"), None);
    }
}

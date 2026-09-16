//! A YAML-writable image of a value, free of serde_json's private number token.

use serde::Serialize;
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde_json::Value as JsonValue;

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
    pub(super) fn of<T: Serialize + ?Sized>(value: &T) -> Result<Self, serde_json::Error> {
        Ok(Self::from_json(&serde_json::to_value(value)?))
    }

    fn from_json(value: &JsonValue) -> Self {
        match value {
            JsonValue::Null => PlainValue::Null,
            JsonValue::Bool(value) => PlainValue::Bool(*value),
            JsonValue::Number(number) => {
                // The widest reading of the lexeme that keeps every digit wins; a number too wide
                // for any of them is one no YAML number could have held either.
                if let Some(value) = number.as_u64() {
                    PlainValue::Unsigned(value)
                } else if let Some(value) = number.as_i64() {
                    PlainValue::Signed(value)
                } else if let Some(value) = number.as_f64() {
                    PlainValue::Float(value)
                } else {
                    PlainValue::Text(number.to_string())
                }
            }
            JsonValue::String(value) => PlainValue::Text(value.clone()),
            JsonValue::Array(members) => {
                PlainValue::Seq(members.iter().map(PlainValue::from_json).collect())
            }
            JsonValue::Object(members) => PlainValue::Map(
                members
                    .iter()
                    .map(|(name, member)| (name.clone(), PlainValue::from_json(member)))
                    .collect(),
            ),
        }
    }
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
}

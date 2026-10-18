//! Opaque JSON data must not acquire serde_json's private scalar meanings.
//!
//! RawValue and the standard serde_json parsers establish the JSON syntax.
//! Inspecting one raw token at a time distinguishes a literal object from
//! serde_json's synthetic number/raw-value maps without reserving user keys.
//! Opaque subtrees retain the normal SDK wire nesting bound. Constructed
//! values beyond that bound receive a deserialization error; this decoder
//! does not inherit a caller's explicitly disabled JSON recursion limit.

use serde::{
    Deserialize, Deserializer,
    de::{Error as _, MapAccess, Visitor},
};
use serde_json::{Value, value::RawValue};
use std::{collections::HashMap, fmt};

// The previous serde boundary accepted 127 containers including its typed
// outer object. Keep the remaining 126-container budget for each opaque field.
const MAX_FIELD_CONTAINERS: u8 = 126;

struct Members<'a>(Vec<(String, &'a RawValue)>);

impl<'de> Deserialize<'de> for Members<'de> {
    fn deserialize<D: Deserializer<'de>>(reader: D) -> Result<Self, D::Error> {
        struct MemberVisitor;
        impl<'de> Visitor<'de> for MemberVisitor {
            type Value = Members<'de>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut fields: A) -> Result<Self::Value, A::Error> {
                let mut members = Vec::new();
                while let Some(member) = fields.next_entry()? {
                    members.push(member);
                }
                Ok(Members(members))
            }
        }
        reader.deserialize_map(MemberVisitor)
    }
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(reader: D) -> Result<Value, D::Error> {
    let raw = Box::<RawValue>::deserialize(reader)?;
    literal_value(&raw, 0).map_err(D::Error::custom)
}

pub(crate) fn deserialize_options<'de, D: Deserializer<'de>>(
    reader: D,
) -> Result<HashMap<String, Value>, D::Error> {
    match deserialize(reader)? {
        Value::Object(fields) => Ok(fields.into_iter().collect()),
        _ => Err(D::Error::custom("expected generation options object")),
    }
}

fn literal_value(raw: &RawValue, depth: u8) -> Result<Value, serde_json::Error> {
    let token = raw.get();
    if matches!(token.as_bytes()[0], b'{' | b'[') && depth >= MAX_FIELD_CONTAINERS {
        return Err(serde_json::Error::custom("recursion limit exceeded"));
    }
    match token.as_bytes()[0] {
        b'{' => {
            let fields: Members<'_> = serde_json::from_str(token)?;
            fields
                .0
                .into_iter()
                .map(|(key, value)| Ok((key, literal_value(value, depth + 1)?)))
                .collect::<Result<_, _>>()
                .map(Value::Object)
        }
        b'[' => {
            let elements: Vec<&RawValue> = serde_json::from_str(token)?;
            elements
                .into_iter()
                .map(|value| literal_value(value, depth + 1))
                .collect::<Result<_, _>>()
                .map(Value::Array)
        }
        b'"' => serde_json::from_str(token).map(Value::String),
        b't' => Ok(Value::Bool(true)),
        b'f' => Ok(Value::Bool(false)),
        b'n' => Ok(Value::Null),
        _ => serde_json::from_str(token).map(Value::Number),
    }
}

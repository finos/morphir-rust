//! Opaque JSON data must not acquire serde_json's private scalar meanings.
//!
//! RawValue and the standard serde_json parsers establish the JSON syntax.
//! After RawValue validates the complete field, materialize its containers in
//! one forward traversal. Public serde_json streaming decoders handle each
//! scalar and object key once. No child container is reparsed as RawValue.
//! Opaque subtrees retain the normal SDK wire nesting bound. Constructed
//! values beyond that bound receive a deserialization error; this decoder
//! does not inherit a caller's explicitly disabled JSON recursion limit.

use serde::{Deserialize, Deserializer, de::Error as _};
use serde_json::{Value, value::RawValue};
use std::collections::HashMap;

// The previous serde boundary accepted 127 containers including its typed
// outer object. Keep the remaining 126-container budget for each opaque field.
const MAX_FIELD_CONTAINERS: u8 = 126;

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(reader: D) -> Result<Value, D::Error> {
    let raw = Box::<RawValue>::deserialize(reader)?;
    morphir_json::read(raw.get(), &mut WireBudget).map_err(|error| match error {
        morphir_json::ReadError::Syntax(error) | morphir_json::ReadError::Budget(error) => {
            D::Error::custom(error)
        }
    })
}

pub(crate) fn deserialize_options<'de, D: Deserializer<'de>>(
    reader: D,
) -> Result<HashMap<String, Value>, D::Error> {
    match deserialize(reader)? {
        Value::Object(fields) => Ok(fields.into_iter().collect()),
        _ => Err(D::Error::custom("expected generation options object")),
    }
}

/// Wire policy intentionally retains the historical opaque container bound.
struct WireBudget;
impl morphir_json::ReadBudget for WireBudget {
    type Error = serde_json::Error;
    fn before_value(
        &mut self,
        depth: usize,
        kind: morphir_json::ValueKind,
    ) -> Result<(), Self::Error> {
        if kind != morphir_json::ValueKind::Scalar && depth >= usize::from(MAX_FIELD_CONTAINERS) {
            return Err(serde_json::Error::custom("recursion limit exceeded"));
        }
        Ok(())
    }
    fn before_string(&mut self, _: &str) -> Result<(), Self::Error> {
        Ok(())
    }
}

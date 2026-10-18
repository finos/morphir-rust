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
    let mut literal = LiteralReader {
        source: raw.get(),
        offset: 0,
    };
    let value = literal.value(0).map_err(D::Error::custom)?;
    literal.whitespace();
    if literal.offset != literal.source.len() {
        return Err(D::Error::custom("trailing literal JSON data"));
    }
    Ok(value)
}

pub(crate) fn deserialize_options<'de, D: Deserializer<'de>>(
    reader: D,
) -> Result<HashMap<String, Value>, D::Error> {
    match deserialize(reader)? {
        Value::Object(fields) => Ok(fields.into_iter().collect()),
        _ => Err(D::Error::custom("expected generation options object")),
    }
}

/// Only constructed from a validated RawValue above. This cursor interprets
/// container structure; serde_json still owns all lexical/syntax validation,
/// string escaping and numeric semantics. Its offset only moves forward.
struct LiteralReader<'a> {
    source: &'a str,
    offset: usize,
}

impl<'a> LiteralReader<'a> {
    fn whitespace(&mut self) {
        while matches!(
            self.source.as_bytes().get(self.offset),
            Some(b' ' | b'\n' | b'\r' | b'\t')
        ) {
            self.offset += 1;
        }
    }

    fn take(&mut self, byte: u8) -> bool {
        self.whitespace();
        if self.source.as_bytes().get(self.offset) == Some(&byte) {
            self.offset += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), serde_json::Error> {
        if self.take(byte) {
            Ok(())
        } else {
            Err(serde_json::Error::custom("invalid literal JSON container"))
        }
    }

    fn scalar<T: Deserialize<'a>>(&mut self) -> Result<T, serde_json::Error> {
        let mut stream =
            serde_json::Deserializer::from_str(&self.source[self.offset..]).into_iter::<T>();
        let value = stream
            .next()
            .ok_or_else(|| serde_json::Error::custom("missing literal JSON scalar"))??;
        self.offset += stream.byte_offset();
        Ok(value)
    }

    fn value(&mut self, depth: u8) -> Result<Value, serde_json::Error> {
        self.whitespace();
        let next = self.source.as_bytes().get(self.offset).copied();
        if matches!(next, Some(b'{' | b'[')) && depth >= MAX_FIELD_CONTAINERS {
            return Err(serde_json::Error::custom("recursion limit exceeded"));
        }
        match next {
            Some(b'{') => {
                self.offset += 1;
                let mut fields = serde_json::Map::new();
                if !self.take(b'}') {
                    loop {
                        let key = self.scalar::<String>()?;
                        self.expect(b':')?;
                        fields.insert(key, self.value(depth + 1)?);
                        if self.take(b'}') {
                            break;
                        }
                        self.expect(b',')?;
                    }
                }
                Ok(Value::Object(fields))
            }
            Some(b'[') => {
                self.offset += 1;
                let mut values = Vec::new();
                if !self.take(b']') {
                    loop {
                        values.push(self.value(depth + 1)?);
                        if self.take(b']') {
                            break;
                        }
                        self.expect(b',')?;
                    }
                }
                Ok(Value::Array(values))
            }
            _ => self.scalar(),
        }
    }
}

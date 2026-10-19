//! One forward delimiter traversal, extracted from the SDK's literal reader.
//! serde_json owns scalar/string lexing; this module owns container punctuation.
use crate::{ReadBudget, ReadError, ValueKind};
use serde::{
    Deserialize, Deserializer,
    de::{DeserializeSeed, Error as _, Visitor},
};
use serde_json::Value;
use std::fmt;

pub(crate) fn read<B: ReadBudget>(
    source: &str,
    budget: &mut B,
) -> Result<Value, ReadError<B::Error>> {
    let mut reader = Reader {
        source,
        offset: 0,
        budget,
    };
    let value = reader.value(0)?;
    reader.whitespace();
    if reader.offset != source.len() {
        return Err(reader.syntax("trailing literal JSON data"));
    }
    Ok(value)
}
struct Reader<'a, 'b, B> {
    source: &'a str,
    offset: usize,
    budget: &'b mut B,
}
impl<'a, B: ReadBudget> Reader<'a, '_, B> {
    fn syntax(&self, message: &str) -> ReadError<B::Error> {
        ReadError::Syntax(serde_json::Error::custom(message))
    }
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
    fn expect(&mut self, byte: u8) -> Result<(), ReadError<B::Error>> {
        if self.take(byte) {
            Ok(())
        } else {
            Err(self.syntax("invalid literal JSON container"))
        }
    }
    fn scalar<T: Deserialize<'a>>(&mut self) -> Result<T, ReadError<B::Error>> {
        let mut stream =
            serde_json::Deserializer::from_str(&self.source[self.offset..]).into_iter::<T>();
        let value = stream
            .next()
            .ok_or_else(|| self.syntax("missing literal JSON scalar"))?
            .map_err(ReadError::Syntax)?;
        self.offset += stream.byte_offset();
        Ok(value)
    }
    fn string(&mut self) -> Result<String, ReadError<B::Error>> {
        let mut deserializer = serde_json::Deserializer::from_str(&self.source[self.offset..]);
        let mut refusal = None;
        let result = StringSeed {
            budget: self.budget,
            refusal: &mut refusal,
        }
        .deserialize(&mut deserializer);
        if let Some(error) = refusal {
            return Err(ReadError::Budget(error));
        }
        let value = result.map_err(ReadError::Syntax)?;
        self.offset += deserializer
            .into_iter::<serde::de::IgnoredAny>()
            .byte_offset();
        Ok(value)
    }
    fn value(&mut self, depth: usize) -> Result<Value, ReadError<B::Error>> {
        self.whitespace();
        let next = self.source.as_bytes().get(self.offset).copied();
        let kind = match next {
            Some(b'{') => ValueKind::Object,
            Some(b'[') => ValueKind::Array,
            _ => ValueKind::Scalar,
        };
        self.budget
            .before_value(depth, kind)
            .map_err(ReadError::Budget)?;
        match next {
            Some(b'{') => {
                self.offset += 1;
                let mut fields = serde_json::Map::new();
                if !self.take(b'}') {
                    loop {
                        self.whitespace();
                        let key = self.string()?;
                        self.expect(b':')?;
                        let value = self.value(depth + 1)?;
                        fields.insert(key, value);
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
                        let value = self.value(depth + 1)?;
                        values.push(value);
                        if self.take(b']') {
                            break;
                        }
                        self.expect(b',')?;
                    }
                }
                Ok(Value::Array(values))
            }
            Some(b'"') => self.string().map(Value::String),
            Some(b'n' | b't' | b'f' | b'-' | b'0'..=b'9') => self.scalar(),
            _ => Err(self.syntax("invalid literal JSON value")),
        }
    }
}
struct StringSeed<'a, B: ReadBudget> {
    budget: &'a mut B,
    refusal: &'a mut Option<B::Error>,
}
impl<'de, B: ReadBudget> DeserializeSeed<'de> for StringSeed<'_, B> {
    type Value = String;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<String, D::Error> {
        deserializer.deserialize_str(self)
    }
}
impl<'de, B: ReadBudget> Visitor<'de> for StringSeed<'_, B> {
    type Value = String;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON string")
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<String, E> {
        if let Err(error) = self.budget.before_string(value) {
            *self.refusal = Some(error);
            return Err(E::custom("literal JSON string budget exceeded"));
        }
        Ok(value.to_owned())
    }
}

// Copyright 2019 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Package-local adaptation of `olpc-cjson` 0.1.4, preserving Unicode code points.
//! See MORPHIR-PROVENANCE.md for source, license and the bounded change.
//!
//! The formatter provides a [`serde_json::Formatter`] to serialize data as [canonical JSON], as
//! defined by OLPC and used in [TUF].
//!
//! [`serde_json::Formatter`]: ../serde_json/ser/trait.Formatter.html
//! [canonical JSON]: http://wiki.laptop.org/go/Canonical_JSON
//! [TUF]: https://theupdateframework.github.io/
//!
//! OLPC's canonical JSON specification is subtly different from other "canonical JSON"
//! specifications, and is also not a strict subset of JSON (specifically, ASCII control characters
//! 0x00&ndash;0x1f are printed literally, which is not valid JSON). Therefore, `serde_json` cannot
//! necessarily deserialize JSON produced by this formatter.
//!
//! This crate is not developed or endorsed by OLPC; use of the term is solely to distinguish this
//! specification of canonical JSON from [other specifications of canonical JSON][xkcd].
//!
//! [xkcd]: https://xkcd.com/927/
//!

#![deny(rust_2018_idioms)]
#![warn(clippy::pedantic)]
#![allow(clippy::must_use_candidate)]

use serde::Serialize;
use serde_json::ser::{CharEscape, CompactFormatter, Formatter, Serializer};
use std::collections::BTreeMap;
use std::io::{Error, ErrorKind, Result, Write};

/// A [`Formatter`] that produces canonical JSON.
///
/// See the [crate-level documentation](../index.html) for more detail.
///
/// [`Formatter`]: ../serde_json/ser/trait.Formatter.html
#[derive(Debug, Default)]
pub struct CanonicalFormatter {
    object_stack: Vec<Object>,
}

/// Internal struct to keep track of an object in progress of being built.
///
/// As keys and values are received by `CanonicalFormatter`, they are written to `next_key` and
/// `next_value` by using the `CanonicalFormatter::writer` convenience method.
///
/// How this struct behaves when `Formatter` methods are called:
///
/// ```plain
/// [other methods]  // values written to the writer received by method
/// begin_object     // create this object
/// /-> begin_object_key    // object.key_done = false;
/// |   [other methods]     // values written to object.next_key, writer received by method ignored
/// |   end_object_key      // object.key_done = true;
/// |   begin_object_value  // [nothing]
/// |   [other methods]     // values written to object.next_value
/// |   end_object_value    // object.next_key and object.next_value are inserted into object.obj
/// \---- // jump back if more values are present
/// end_object       // write the object (sorted by its keys) to the writer received by the method
/// ```
#[derive(Debug, Default)]
struct Object {
    obj: BTreeMap<Vec<u8>, ObjectEntry>,
    next_key: Vec<u8>,
    next_value: Vec<u8>,
    key_done: bool,
}

#[derive(Debug)]
struct ObjectEntry {
    encoded_key: Vec<u8>,
    value: Vec<u8>,
}

/// Undo only this formatter's two escapes, retaining literal controls and UTF-8.
/// Canonical strings are not necessarily JSON, so a JSON parser cannot do this.
fn decoded_key(encoded: &[u8]) -> Result<Vec<u8>> {
    let inner = encoded
        .strip_prefix(b"\"")
        .and_then(|key| key.strip_suffix(b"\""))
        .ok_or_else(|| Error::other("canonical object key is not a string"))?;
    let mut decoded = Vec::with_capacity(inner.len());
    let mut bytes = inner.iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == b'\\' {
            match bytes.next() {
                Some(escaped @ (b'\\' | b'"')) => decoded.push(escaped),
                _ => return Err(Error::other("invalid canonical object key escape")),
            }
        } else {
            decoded.push(byte);
        }
    }
    Ok(decoded)
}

impl CanonicalFormatter {
    /// Create a new `CanonicalFormatter` object.
    pub fn new() -> Self {
        Self::default()
    }

    /// Convenience method to return the appropriate writer given the current context.
    ///
    /// If we are currently writing an object (that is, if `!self.object_stack.is_empty()`), we
    /// need to write the value to either the next key or next value depending on that state
    /// machine. See the docstrings for `Object` for more detail.
    ///
    /// If we are not currently writing an object, pass through `writer`.
    fn writer<'a, W: Write + ?Sized>(&'a mut self, writer: &'a mut W) -> Box<dyn Write + 'a> {
        self.object_stack
            .last_mut()
            .map_or(Box::new(writer), |object| {
                if object.key_done {
                    Box::new(&mut object.next_value)
                } else {
                    Box::new(&mut object.next_key)
                }
            })
    }

    /// Returns a mutable reference to the top of the object stack.
    fn obj_mut(&mut self) -> Result<&mut Object> {
        self.object_stack.last_mut().ok_or_else(|| {
            Error::other("serde_json called an object method without calling begin_object first")
        })
    }
}

/// Wraps `serde_json::CompactFormatter` to use the appropriate writer (see
/// `CanonicalFormatter::writer`).
macro_rules! wrapper {
    ($f:ident) => {
        fn $f<W: Write + ?Sized>(&mut self, writer: &mut W) -> Result<()> {
            CompactFormatter.$f(&mut self.writer(writer))
        }
    };

    ($f:ident, $t:ty) => {
        fn $f<W: Write + ?Sized>(&mut self, writer: &mut W, arg: $t) -> Result<()> {
            CompactFormatter.$f(&mut self.writer(writer), arg)
        }
    };
}

/// This is used in three places. Write it once.
macro_rules! float_err {
    () => {
        Err(Error::new(
            ErrorKind::InvalidInput,
            "floating point numbers are not allowed in canonical JSON",
        ))
    };
}

impl Formatter for CanonicalFormatter {
    wrapper!(write_null);
    wrapper!(write_bool, bool);
    wrapper!(write_i8, i8);
    wrapper!(write_i16, i16);
    wrapper!(write_i32, i32);
    wrapper!(write_i64, i64);
    wrapper!(write_i128, i128);
    wrapper!(write_u8, u8);
    wrapper!(write_u16, u16);
    wrapper!(write_u32, u32);
    wrapper!(write_u64, u64);
    wrapper!(write_u128, u128);

    fn write_f32<W: Write + ?Sized>(&mut self, _writer: &mut W, _value: f32) -> Result<()> {
        float_err!()
    }

    fn write_f64<W: Write + ?Sized>(&mut self, _writer: &mut W, _value: f64) -> Result<()> {
        float_err!()
    }

    // By default this is only used for u128/i128. If serde_json's `arbitrary_precision` feature is
    // enabled, all numbers are internally stored as strings, and this method is always used (even
    // for floating point values).
    fn write_number_str<W: Write + ?Sized>(&mut self, writer: &mut W, value: &str) -> Result<()> {
        if value.chars().any(|c| c == '.' || c == 'e' || c == 'E') {
            float_err!()
        } else {
            CompactFormatter.write_number_str(&mut self.writer(writer), value)
        }
    }

    wrapper!(begin_string);
    wrapper!(end_string);

    // Preserve the signed string code points exactly; the package profile forbids normalization.
    fn write_string_fragment<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
        fragment: &str,
    ) -> Result<()> {
        self.writer(writer).write_all(fragment.as_bytes())
    }

    // Only quotes and backslashes are escaped in canonical JSON.
    fn write_char_escape<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
        char_escape: CharEscape,
    ) -> Result<()> {
        match char_escape {
            CharEscape::Quote | CharEscape::ReverseSolidus => {
                self.writer(writer).write_all(b"\\")?;
            }
            _ => {}
        }
        self.writer(writer).write_all(&[match char_escape {
            CharEscape::Quote => b'\"',
            CharEscape::ReverseSolidus => b'\\',
            CharEscape::Solidus => b'/',
            CharEscape::Backspace => b'\x08',
            CharEscape::FormFeed => b'\x0c',
            CharEscape::LineFeed => b'\n',
            CharEscape::CarriageReturn => b'\r',
            CharEscape::Tab => b'\t',
            CharEscape::AsciiControl(byte) => byte,
        }])
    }

    wrapper!(begin_array);
    wrapper!(end_array);
    wrapper!(begin_array_value, bool); // hack: this passes through the `first` argument
    wrapper!(end_array_value);

    // Here are the object methods. Because keys must be sorted, we serialize the object's keys and
    // values in memory as a `BTreeMap`, then write it all out when `end_object_value` is called.

    fn begin_object<W: Write + ?Sized>(&mut self, writer: &mut W) -> Result<()> {
        CompactFormatter.begin_object(&mut self.writer(writer))?;
        self.object_stack.push(Object::default());
        Ok(())
    }

    fn end_object<W: Write + ?Sized>(&mut self, writer: &mut W) -> Result<()> {
        let object = self.object_stack.pop().ok_or_else(|| {
            Error::other(
                "serde_json called Formatter::end_object object method
                 without calling begin_object first",
            )
        })?;
        let mut writer = self.writer(writer);
        let mut first = true;

        for entry in object.obj.into_values() {
            CompactFormatter.begin_object_key(&mut writer, first)?;
            writer.write_all(&entry.encoded_key)?;
            CompactFormatter.end_object_key(&mut writer)?;

            CompactFormatter.begin_object_value(&mut writer)?;
            writer.write_all(&entry.value)?;
            CompactFormatter.end_object_value(&mut writer)?;

            first = false;
        }

        CompactFormatter.end_object(&mut writer)
    }

    fn begin_object_key<W: Write + ?Sized>(&mut self, _writer: &mut W, _first: bool) -> Result<()> {
        let object = self.obj_mut()?;
        object.key_done = false;
        Ok(())
    }

    fn end_object_key<W: Write + ?Sized>(&mut self, _writer: &mut W) -> Result<()> {
        let object = self.obj_mut()?;
        object.key_done = true;
        Ok(())
    }

    fn begin_object_value<W: Write + ?Sized>(&mut self, _writer: &mut W) -> Result<()> {
        Ok(())
    }

    fn end_object_value<W: Write + ?Sized>(&mut self, _writer: &mut W) -> Result<()> {
        let object = self.obj_mut()?;
        let key = std::mem::take(&mut object.next_key);
        let value = std::mem::take(&mut object.next_value);
        // UTF-8 byte ordering agrees with Unicode scalar ordering for valid keys.
        object.obj.insert(
            decoded_key(&key)?,
            ObjectEntry {
                encoded_key: key,
                value,
            },
        );
        Ok(())
    }

    // This is for serde_json's `raw_value` feature, which provides a RawValue type that is passed
    // through as-is. That's not good enough for canonical JSON, so we parse it and immediately
    // write it back out... as canonical JSON.
    fn write_raw_fragment<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
        fragment: &str,
    ) -> Result<()> {
        let mut ser = Serializer::with_formatter(self.writer(writer), Self::new());
        serde_json::from_str::<serde_json::Value>(fragment)?.serialize(&mut ser)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;

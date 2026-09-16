//! Bounded printable-ASCII package metadata normalization.

use crate::{InvalidDocument, digest::Digest, strict_json};
use serde_json::Value;

/// Metadata that has passed the draft's syntax, character and depth checks.
///
/// ```
/// use morphir_package::metadata::NormalizedMetadata;
/// let metadata = NormalizedMetadata::parse(r#"{ "z": [], "a": "x" }"#)?;
/// assert_eq!(metadata.as_str(), r#"{"a":"x","z":[]}"#);
/// # Ok::<(), morphir_package::InvalidDocument>(())
/// ```
#[derive(Debug, Clone)]
pub struct NormalizedMetadata {
    canonical: String,
    value: Value,
}

impl NormalizedMetadata {
    /// Parse and normalize JSON. Root depth is zero; at most 64 edges are allowed.
    pub fn parse(input: &str) -> Result<Self, InvalidDocument> {
        let value = strict_json::parse(input).map_err(|_| InvalidDocument)?;
        let mut canonical = String::new();
        write(&value, 0, &mut canonical)?;
        Ok(Self { canonical, value })
    }

    /// Compact canonical text, without a BOM or final newline.
    pub fn as_str(&self) -> &str {
        &self.canonical
    }
    /// The validated metadata tree.
    pub fn value(&self) -> &Value {
        &self.value
    }
    /// SHA-256 over canonical metadata bytes.
    pub fn manifest_digest(&self) -> Digest {
        Digest::of_bytes(self.canonical.as_bytes())
    }
    /// Domain-separated SHA-256 over canonical metadata bytes.
    pub fn content_digest(&self) -> Digest {
        Digest::of_parts(&[
            b"morphir-package-content:0.1.0-draft.1\n",
            self.canonical.as_bytes(),
        ])
    }
}

fn string(text: &str, output: &mut String) -> Result<(), InvalidDocument> {
    if !text.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        return Err(InvalidDocument);
    }
    output.push('"');
    for ch in text.chars() {
        if matches!(ch, '"' | '\\') {
            output.push('\\');
        }
        output.push(ch);
    }
    output.push('"');
    Ok(())
}

fn write(value: &Value, depth: usize, output: &mut String) -> Result<(), InvalidDocument> {
    if depth > 64 {
        return Err(InvalidDocument);
    }
    match value {
        Value::String(text) => string(text, output),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write(value, depth + 1, output)?;
            }
            output.push(']');
            Ok(())
        }
        Value::Object(members) => {
            output.push('{');
            let mut keys: Vec<_> = members.keys().collect();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                string(key, output)?;
                output.push(':');
                write(&members[key], depth + 1, output)?;
            }
            output.push('}');
            Ok(())
        }
        _ => Err(InvalidDocument),
    }
}

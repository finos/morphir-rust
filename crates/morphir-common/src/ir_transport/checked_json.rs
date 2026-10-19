//! Opt-in checked generic JSON projection for model-originals.
//!
//! This is the JSON direct-Value proof only. Historical v1 storage decoding is
//! unchanged. Typed v3, YAML, Ion and document-tree checked assembly remain
//! pending. A caller must provide a frozen source snapshot, own output retention
//! credit, and discard any output prefix on error. Source/assembly byte counts
//! do not claim exact heap/RSS use or qualify tokenizer storage in other codecs.
//!
//! ```
//! use morphir_common::ir_transport::checked_json::{write_literal_projection, ProjectionByteLimit};
//! use morphir_core::ir::decode::{DecodeBudget, DecodeLimits};
//! let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
//! let mut private_output = Vec::new();
//! let _headers = write_literal_projection(br#"{"formatVersion":4,"extra":true}"#,
//!     &mut budget, &mut private_output, ProjectionByteLimit::model_originals())?;
//! assert!(!private_output.is_empty());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
use morphir_core::format_version::{NormalizedFormatVersion, ScalarValue, SupportTable};
use morphir_core::ir::decode::{DecodeBudget, JsonDecodeError, check_assembled, read_json};
use std::io::Write;
/// Actual release evidence, independent of a task descriptor.
#[derive(Clone, Debug, PartialEq)]
pub enum HeaderEvidence {
    /// Recognized actual header, retaining support-table compatibility.
    Reported(NormalizedFormatVersion),
    /// No recognizable actual header; generic JSON acceptance is unchanged.
    NotReported,
}
/// Source and completed projection evidence are distinct positions.
#[derive(Clone, Debug, PartialEq)]
pub struct JsonProjectionHeaders {
    source: HeaderEvidence,
    projection: HeaderEvidence,
}
impl JsonProjectionHeaders {
    /// Header derived from this frozen source's effective JSON member.
    pub fn source(&self) -> &HeaderEvidence {
        &self.source
    }
    /// Header of the completed serialized representation.
    pub fn projection(&self) -> &HeaderEvidence {
        &self.projection
    }
}

/// A finite serialized projection byte limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectionByteLimit(usize);
impl ProjectionByteLimit {
    /// The inclusive 64 MiB serialized projection ceiling.
    pub const fn model_originals() -> Self {
        Self(64 * 1024 * 1024)
    }
    /// Select a positive limit no greater than 64 MiB.
    pub fn new(bytes: usize) -> Option<Self> {
        (bytes > 0 && bytes <= 64 * 1024 * 1024).then_some(Self(bytes))
    }
}
/// Generic source, assembly, writer or output refusal.
#[derive(Debug)]
pub enum JsonProjectionError {
    /// Checked decoding refused.
    Decode(JsonDecodeError),
    /// Serialized projection limit exceeded.
    OutputLimit,
    /// Serialization or output IO failed.
    Output(serde_json::Error),
}
/// Write generic literal source JSON, retaining unknown members.
///
/// The same checked Value supplies source facts and serialized output; no
/// probe/decode double charge or typed IR validation is introduced. Unreported
/// header evidence does not reject otherwise accepted JSON. Source and output
/// header positions remain distinct even though this route does not migrate.
/// Output credit is checked before writes; a failed operation can leave a
/// prefix in the supplied writer, which must remain private and be discarded.
pub fn write_literal_projection(
    source: &[u8],
    budget: &mut DecodeBudget,
    output: &mut dyn Write,
    limit: ProjectionByteLimit,
) -> Result<JsonProjectionHeaders, JsonProjectionError> {
    let value = read_json(source, budget).map_err(JsonProjectionError::Decode)?;
    check_assembled(&value, budget)
        .map_err(|e| JsonProjectionError::Decode(JsonDecodeError::Limit(e)))?;
    let mut bounded = BoundedWriter {
        output,
        remaining: limit.0,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut bounded, &value);
    if bounded.exceeded {
        return Err(JsonProjectionError::OutputLimit);
    }
    result.map_err(JsonProjectionError::Output)?;
    let source = actual_header(&value);
    // This generic route serializes the same Value without version migration.
    // Future typed routes must establish their own actual output-header fact.
    let projection = actual_header(&value);
    Ok(JsonProjectionHeaders { source, projection })
}

struct BoundedWriter<'a> {
    output: &'a mut dyn Write,
    remaining: usize,
    exceeded: bool,
}
impl Write for BoundedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.remaining {
            self.exceeded = true;
            return Err(std::io::Error::other("projection byte limit exceeded"));
        }
        let written = self.output.write(bytes)?;
        self.remaining -= written;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}

fn actual_header(value: &serde_json::Value) -> HeaderEvidence {
    // A valid release has three non-padded u32 decimal components and two dots.
    // Reject an impossible string before ScalarValue's owned string clone.
    const MAX_TRIPLET_TEXT_BYTES: usize = 3 * 10 + 2;
    if value
        .as_object()
        .and_then(|fields| fields.get("formatVersion"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|text| text.len() > MAX_TRIPLET_TEXT_BYTES)
    {
        return HeaderEvidence::NotReported;
    }
    value
        .as_object()
        .and_then(|fields| fields.get("formatVersion"))
        .and_then(|value| ScalarValue::from_json(value).ok())
        .and_then(|scalar| {
            NormalizedFormatVersion::from_scalar(&scalar, &SupportTable::reference()).ok()
        })
        .map(HeaderEvidence::Reported)
        .unwrap_or(HeaderEvidence::NotReported)
}

impl std::fmt::Display for JsonProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(error) => error.fmt(f),
            Self::OutputLimit => f.write_str("serialized JSON projection exceeds its byte limit"),
            Self::Output(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for JsonProjectionError {}

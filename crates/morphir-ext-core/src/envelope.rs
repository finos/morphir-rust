//! Envelope protocol for Morphir extensions.
//!
//! All messages between host, runtime, and programs use an Envelope structure
//! containing a header, content type, and raw bytes content.

use serde::{Deserialize, Serialize};

/// Header contains metadata for envelope routing and tracking.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Header {
    /// Sequence number for tracking message order.
    #[serde(default)]
    pub seqnum: u64,
    /// Session identifier for grouping related messages.
    #[serde(default)]
    pub session_id: String,
    /// Optional kind/type hint for the envelope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// Envelope wraps all extension messages with metadata and typed content.
///
/// The envelope protocol is the universal message format for Morphir extensions,
/// supporting both design-time and runtime operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    /// Message header with metadata.
    pub header: Header,
    /// MIME type describing the content format (e.g., "application/json").
    pub content_type: String,
    /// Raw content bytes.
    #[serde(with = "serde_bytes")]
    pub content: Vec<u8>,
}

impl Envelope {
    /// Create a new envelope with JSON content.
    pub fn json<T: Serialize>(value: &T) -> Result<Self, serde_json::Error> {
        Ok(Self {
            header: Header::default(),
            content_type: "application/json".to_string(),
            content: serde_json::to_vec(value)?,
        })
    }

    /// Parse the envelope content as JSON.
    pub fn as_json<T: for<'de> Deserialize<'de>>(&self) -> Result<T, EnvelopeError> {
        if self.content_type != "application/json" {
            return Err(EnvelopeError::ContentTypeMismatch {
                expected: "application/json".to_string(),
                actual: self.content_type.clone(),
            });
        }
        serde_json::from_slice(&self.content).map_err(EnvelopeError::JsonError)
    }

    /// Create an envelope with custom content type and bytes.
    pub fn new(content_type: impl Into<String>, content: Vec<u8>) -> Self {
        Self {
            header: Header::default(),
            content_type: content_type.into(),
            content,
        }
    }

    /// Create an envelope with a custom header.
    pub fn with_header(mut self, header: Header) -> Self {
        self.header = header;
        self
    }
}

/// Errors that can occur when working with envelopes.
#[derive(Debug)]
pub enum EnvelopeError {
    /// Content type doesn't match expected type.
    ContentTypeMismatch { expected: String, actual: String },
    /// JSON serialization/deserialization error.
    JsonError(serde_json::Error),
}

impl std::fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvelopeError::ContentTypeMismatch { expected, actual } => {
                write!(f, "expected content type {}, got {}", expected, actual)
            }
            EnvelopeError::JsonError(e) => write!(f, "JSON error: {}", e),
        }
    }
}

impl std::error::Error for EnvelopeError {}

/// Encode an envelope to JSON bytes.
pub fn encode_envelope(env: &Envelope) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(env)
}

/// Decode an envelope from JSON bytes.
pub fn decode_envelope(bytes: &[u8]) -> Result<Envelope, serde_json::Error> {
    serde_json::from_slice(bytes)
}

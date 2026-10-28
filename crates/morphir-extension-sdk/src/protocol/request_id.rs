//! Canonical MEP request identity, encoded as a JSON number or string.

use serde::{Deserialize, Serialize};

/// A non-negative numeric or string JSON-RPC request identifier.
///
/// String IDs retain their exact text and never become numeric IDs.
///
/// ```
/// use morphir_extension_sdk::protocol::RequestId;
/// let id = RequestId::from("007");
/// assert_eq!(serde_json::to_string(&id).unwrap(), "\"007\"");
/// assert_ne!(id, RequestId::from(7_u64));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    /// A canonical non-negative integer, including the full u64 range.
    Numeric(u64),
    /// An opaque string whose type and value are preserved.
    String(String),
}

impl From<u64> for RequestId {
    fn from(value: u64) -> Self {
        Self::Numeric(value)
    }
}

impl From<String> for RequestId {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for RequestId {
    fn from(value: &str) -> Self {
        Self::String(value.into())
    }
}

impl PartialEq<u64> for RequestId {
    fn eq(&self, other: &u64) -> bool {
        matches!(self, Self::Numeric(value) if value == other)
    }
}

impl PartialEq<RequestId> for u64 {
    fn eq(&self, other: &RequestId) -> bool {
        other == self
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Numeric(value) => value.fmt(f),
            Self::String(value) => value.fmt(f),
        }
    }
}

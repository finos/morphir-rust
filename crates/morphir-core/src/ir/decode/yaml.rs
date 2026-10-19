//! Checked physical YAML parsing, preserving the existing Morphir YAML profile.
use super::{DecodeBudget, DecodeLimitExceeded};
use crate::ir::Diagnostic;
use serde_json::Value;
use std::{error::Error, fmt};

/// A source Unicode, existing YAML profile, or finite resource refusal.
#[derive(Debug)]
pub enum YamlDecodeError {
    /// The source is not UTF-8.
    Utf8(std::str::Utf8Error),
    /// The existing YAML profile refused the source.
    Profile(Diagnostic),
    /// An aggregate source resource limit refused further materialization.
    Limit(DecodeLimitExceeded),
}
impl fmt::Display for YamlDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Utf8(error) => error.fmt(f),
            Self::Profile(error) => write!(f, "{:?}: {}", error.code, error.message),
            Self::Limit(error) => error.fmt(f),
        }
    }
}
impl Error for YamlDecodeError {}

/// Parse one primary YAML document with the same aggregate source ledger as JSON.
///
/// ```
/// use morphir_core::ir::decode::{DecodeBudget, DecodeLimits, read_yaml};
/// let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
/// let value = read_yaml(b"formatVersion: 4\nopaque: [true, null]\n", &mut budget)?;
/// assert_eq!(value["formatVersion"].as_u64(), Some(4));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// Bytes are reserved before UTF-8 parsing. Values are reserved before frame
/// pushes, scalar resolution and insertion; decoded keys and string values
/// are reserved before their AST ownership. Parser flow/block nesting limits
/// are set before parsing, and comment capture is disabled. Granit's scalar
/// scratch is bounded by source bytes, separately from retained string credit.
/// This is physical profile parsing, not typed v3/tree assembly or a completed
/// semantic projection. Existing unbounded-policy YAML entrypoints are unchanged.
pub fn read_yaml(source: &[u8], budget: &mut DecodeBudget) -> Result<Value, YamlDecodeError> {
    budget
        .reserve_source(source.len())
        .map_err(YamlDecodeError::Limit)?;
    let text = std::str::from_utf8(source).map_err(YamlDecodeError::Utf8)?;
    crate::ir::yaml::read_bounded(text, budget.source()).map_err(|error| match error {
        crate::ir::yaml::ReadFailure::Profile(error) => YamlDecodeError::Profile(error),
        crate::ir::yaml::ReadFailure::Limit(error) => YamlDecodeError::Limit(error),
    })
}

//! Opt-in bounded physical JSON and YAML for the additive model-originals profile.
//!
//! Historical JSON storage and extension-wire decoders are unchanged. This
//! JSON policy preserves private-looking keys literally, ordinary last-wins
//! object members and active numeric precision/map ordering. YAML retains its
//! existing profile semantics, including duplicate-key refusal.
//!
//! ```
//! use morphir_core::ir::decode::{DecodeBudget, DecodeLimits, read_json};
//! let mut budget = DecodeBudget::new(DecodeLimits::model_originals());
//! let value = read_json(br#"{"extra":{"$serde_json::private::Number":"1"}}"#, &mut budget)?;
//! assert!(value["extra"].is_object());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
mod budget;
pub(crate) use budget::PhaseBudget;
mod yaml;
pub use budget::{
    DecodeBudget, DecodeLimit, DecodeLimitExceeded, DecodeLimits, DecodePhase, DecodeUsage,
    InvalidDecodeLimits,
};
use serde_json::Value;
use std::{error::Error, fmt};
pub use yaml::{YamlDecodeError, read_yaml};

/// Syntax, Unicode or finite resource refusal, without retained source bytes.
#[derive(Debug)]
pub enum JsonDecodeError {
    /// Source is not UTF-8 JSON.
    Utf8(std::str::Utf8Error),
    /// Invalid source JSON syntax.
    Syntax(serde_json::Error),
    /// A source budget refused before materialization, or an assembled postcondition refused.
    Limit(DecodeLimitExceeded),
}
impl fmt::Display for JsonDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Utf8(e) => e.fmt(f),
            Self::Syntax(e) => e.fmt(f),
            Self::Limit(e) => e.fmt(f),
        }
    }
}
impl Error for JsonDecodeError {}

/// Decode one primary JSON source document with aggregate source limits.
///
/// Byte credit is reserved before parsing. Value credit precedes container or
/// scalar materialization. Decoded string credit precedes the owned string
/// copy; serde's escaped-string scratch remains bounded by source bytes.
/// This is a generic JSON value route, not typed IR validation or a strict RPC
/// parser. Ordinary duplicate members replace earlier members, but every
/// visited value/string is still charged. No BOM stripping is added.
pub fn read_json(source: &[u8], budget: &mut DecodeBudget) -> Result<Value, JsonDecodeError> {
    budget
        .reserve_source(source.len())
        .map_err(JsonDecodeError::Limit)?;
    let text = std::str::from_utf8(source).map_err(JsonDecodeError::Utf8)?;
    morphir_json::read(text, budget.source()).map_err(|error| match error {
        morphir_json::ReadError::Syntax(e) => JsonDecodeError::Syntax(e),
        morphir_json::ReadError::Budget(e) => JsonDecodeError::Limit(e),
    })
}

/// Check the assembled representation in its independent phase before output.
///
/// This is a postcondition over an already owned Value. It does not prove
/// pre-growth enforcement for a typed/tree/event assembler. Those assemblers
/// still need their own before-growth hooks. The generic JSON projection reuses
/// its source Value and performs no second model materialization.
///
/// Borrowed traversal does not clone values or allocate a pending-node vector.
/// It stops before entering a forbidden child. Call once per assembled result;
/// it does not debit the source ledger or reset previous assembly usage.
pub fn check_assembled(
    value: &Value,
    budget: &mut DecodeBudget,
) -> Result<(), DecodeLimitExceeded> {
    use morphir_json::{ReadBudget, ValueKind};
    fn visit(
        value: &Value,
        depth: usize,
        phase: &mut budget::PhaseBudget,
    ) -> Result<(), DecodeLimitExceeded> {
        let kind = match value {
            Value::Array(_) => ValueKind::Array,
            Value::Object(_) => ValueKind::Object,
            _ => ValueKind::Scalar,
        };
        phase.before_value(depth, kind)?;
        match value {
            Value::Array(items) => {
                for item in items {
                    visit(item, depth + 1, phase)?;
                }
            }
            Value::Object(fields) => {
                for (key, item) in fields {
                    phase.before_string(key)?;
                    visit(item, depth + 1, phase)?;
                }
            }
            Value::String(value) => phase.before_string(value)?,
            _ => (),
        }
        Ok(())
    }
    visit(value, 0, budget.assembled())
}

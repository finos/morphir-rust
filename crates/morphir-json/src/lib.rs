//! Literal JSON values without serde-private key interpretation.
//!
//! A caller supplies finite resource checks. Containers use one forward
//! delimiter traversal extracted from the extension SDK; public serde_json
//! decoders own string/scalar syntax. No private-looking key is special.
//! Ordinary duplicate members retain last-wins semantics, active map ordering
//! and active numeric precision. This is not the strict RPC framing policy.
//!
//! ```
//! use morphir_json::{ReadBudget, ValueKind, read};
//! struct Bounded { values: usize }
//! impl ReadBudget for Bounded {
//!     type Error = &'static str;
//!     fn before_value(&mut self, depth: usize, _: ValueKind) -> Result<(), Self::Error> {
//!         if depth > 8 || self.values == 0 { return Err("limit"); }
//!         self.values -= 1;
//!         Ok(())
//!     }
//!     fn before_string(&mut self, text: &str) -> Result<(), Self::Error> {
//!         if text.len() > 1024 { Err("string limit") } else { Ok(()) }
//!     }
//! }
//! let value = read(r#"{"$serde_json::private::Number":"1"}"#,
//!     &mut Bounded { values: 4 }).unwrap();
//! assert!(value.is_object());
//! ```
use serde_json::Value;
mod reader;

/// The syntax kind about to be materialized.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueKind {
    /// An object.
    Object,
    /// An array.
    Array,
    /// A scalar.
    Scalar,
}
/// Resource policy invoked before owned values or strings are built.
pub trait ReadBudget {
    /// The caller's typed refusal.
    type Error;
    /// Reserve one value at root-zero depth.
    fn before_value(&mut self, depth: usize, kind: ValueKind) -> Result<(), Self::Error>;
    /// Reserve decoded key or string units before copying into owned storage.
    fn before_string(&mut self, value: &str) -> Result<(), Self::Error>;
}
/// A syntax error or a caller-owned resource refusal.
#[derive(Debug)]
pub enum ReadError<E> {
    /// Invalid JSON syntax.
    Syntax(serde_json::Error),
    /// Resource policy refusal.
    Budget(E),
}
/// Read one complete literal JSON value.
///
/// The caller must bound source bytes before calling and refuse excessive
/// depth/values through ReadBudget. Value checks precede parsing/materializing
/// that value; string checks precede owned copies. Escaped-string scratch is
/// serde-owned and bounded by the caller's source-byte cap, not a heap limit.
/// Refusal consumes any reservations already made; this function never resets
/// caller accounting or falls back to another interpretation.
pub fn read<B: ReadBudget>(source: &str, budget: &mut B) -> Result<Value, ReadError<B::Error>> {
    reader::read(source, budget)
}

impl<E: std::fmt::Display> std::fmt::Display for ReadError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Syntax(error) => error.fmt(f),
            Self::Budget(error) => error.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for ReadError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Syntax(error) => Some(error),
            Self::Budget(error) => Some(error),
        }
    }
}

//! Morphir Extension crate
//!
//! Actor-based runtime for Morphir extensions using Kameo.

pub mod actor;
pub mod runtime;

// Re-export main types
pub use runtime::{EnvValue, ExtensionInstance, ExtensionRuntime, LogLevel, WitEnvelope};

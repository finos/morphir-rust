//! Bounded draft.3 local-registry interpretation and preflight helpers.
//!
//! Decoding does not authenticate a release. Publisher evidence proves signatures
//! only; assurance receipts record a fresh host preflight, never filesystem authority.

/// Fresh-host preflight and serializable assurance records.
pub mod assurance;
mod diagnostics;
mod domain;
mod json;
mod lock;
mod metadata;
mod policy;
mod publisher;
mod shape;

pub use diagnostics::*;
pub use domain::*;
pub use json::*;
pub use lock::*;
pub use metadata::*;
pub use policy::*;
pub use publisher::*;

/// The experimental local-registry wire contract.
pub const CONTRACT_VERSION: &str = "0.1.0-draft.3";

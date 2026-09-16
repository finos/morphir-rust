//! Experimental Library package operations for `0.1.0-draft.1`.
//!
//! This crate checks supplied metadata and closed Library sets. It does not resolve
//! dependencies, acquire files, or establish trust or public API compatibility.

pub mod digest;
pub mod library;
pub mod metadata;
pub mod schema;
pub mod strict_json;

/// A document is outside the draft profile or inconsistent with its declarations.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("invalid package document")]
pub struct InvalidDocument;

/// The exact experimental package contract implemented here.
pub const CONTRACT_VERSION: &str = "0.1.0-draft.1";

//! Morphir Models - Core IR model definitions
//!
//! This crate provides the core data structures and utilities for working with
//! Morphir IR (Intermediate Representation) in a functional, type-safe manner.

pub mod error;
pub mod ir;

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::error::{Error, Result};
    pub use crate::ir::*;
}


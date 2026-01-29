//! Morphir Classic IR (Elm-compatible V1-V3)
//!
//! This module implements the Morphir IR structures exactly as they appear in the
//! `morphir-elm` reference implementation. It uses recursive types with generic
//! attributes and supports the specific array-based JSON serialization format used
//! by the classic Morphir tools.

// Core types
pub mod access;
pub mod documented;
pub mod literal;
pub mod naming;
pub mod pattern;
pub mod types;
pub mod value;

// Structure types
pub mod distribution;
pub mod module;
pub mod package;

// Re-exports for convenience
pub use access::{Access, AccessControlled};
pub use distribution::{Distribution, DistributionBody, LibraryTag};
pub use documented::Documented;
pub use literal::Literal;
pub use module::{Module, ModuleDefinition, ModuleDetail, ModuleEntry};
pub use naming::{FQName, Name, Path};
pub use package::Package;
pub use pattern::Pattern;
pub use types::{Constructor, Field, Type, TypeDefinition, TypeSpecification};
pub use value::{Definition, Value, ValueDefinition, ValueSpecification};



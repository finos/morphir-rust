//! Morphir IR module.
//!
//! V4 is the primary/canonical format. Classic (V1-V3) is a submodule for legacy support.
//!
//! # Type Aliases
//!
//! For V4 (preferred), use `Type`, `Value`, `Pattern` directly with `TypeAttributes`
//! and `ValueAttributes`. Or use the convenience aliases `TypeExpr` and `ValueExpr`.
//!
//! For Classic (V1-V3 compatibility), use the `classic` submodule directly:
//! - `ir::classic::Type<A>`, `ir::classic::Value<TA, VA>`, etc.

// V4 Core Types (Primary)
pub mod attributes;
pub mod literal;
pub mod pattern;
pub mod serde_tagged;
pub mod serde_v4;
pub mod type_def;
pub mod type_expr;
pub mod value_expr;

// Legacy support
pub mod classic;
pub mod v4;

// Re-exports for V4 types (primary)
pub use attributes::{
    SourceLocation,
    TypeAttributes,
    // Convenience aliases
    TypeExpr,
    ValueAttributes,
    ValueExpr,
};
pub use literal::Literal;
pub use pattern::Pattern;
pub use type_def::{AccessControlled, Constructor, ConstructorArg, Incompleteness, TypeDefinition};
pub use type_expr::{Field, Type};
pub use value_expr::{
    HoleReason, InputType, LetBinding, NativeHint, NativeInfo, PatternCase, RecordFieldEntry,
    Value, ValueBody, ValueDefinition,
};

// Re-export classic types for backward compatibility (temporary)
pub use classic::*;

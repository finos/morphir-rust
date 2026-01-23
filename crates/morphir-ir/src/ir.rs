//! Morphir IR module.
//!
//! V4 is the primary/canonical format. Classic (V1-V3) is a submodule for legacy support.
//!
//! # Type Aliases
//!
//! For V4 (preferred), use `Type`, `Value`, `Pattern` directly (defaults to V4 attributes).
//! Or use the convenience aliases `TypeExpr` and `ValueExpr`.
//!
//! For Classic (V1-V3 compatibility), use the `Classic` prefix:
//! - [`ClassicType`], [`ClassicValue`], [`ClassicPattern`], etc.

// V4 Core Types (Primary)
pub mod literal;
pub mod type_expr;
pub mod pattern;
pub mod value_expr;
pub mod type_def;
pub mod attributes;
pub mod serde_tagged;
pub mod serde_v4;

// Legacy support
pub mod classic;
pub mod v4;

// Re-exports for V4 types (primary)
pub use literal::Literal;
pub use type_expr::{Field, Type};
pub use pattern::Pattern;
pub use value_expr::{
    HoleReason, InputType, LetBinding, NativeHint, NativeInfo, PatternCase,
    RecordFieldEntry, Value, ValueBody, ValueDefinition,
};
pub use type_def::{
    AccessControlled, Constructor, ConstructorArg, Incompleteness, TypeDefinition,
};
pub use attributes::{
    SourceLocation, TypeAttributes, ValueAttributes,
    // Convenience aliases
    TypeExpr, ValueExpr,
    // Classic type aliases (for V1-V3 compatibility)
    ClassicAttrs, ClassicType, ClassicField, ClassicPattern, ClassicValue,
    ClassicValueDefinition, ClassicTypeDefinition,
};

// Re-export classic types for backward compatibility (temporary)
pub use classic::*;

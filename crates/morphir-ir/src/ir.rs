//! Morphir IR module.
//!
//! V4 is the primary/canonical format. Classic (V1-V3) is a submodule for legacy support.
//!
//! # Type Aliases
//!
//! For V4 (preferred), use the type aliases with `V4` prefix:
//! - [`V4Type`], [`V4Value`], [`V4Pattern`], etc.
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

// Legacy support
pub mod classic;
pub mod v4;

// Re-exports for V4 types (primary)
pub use literal::Literal;
pub use type_expr::{Field, Type};
pub use pattern::Pattern;
pub use value_expr::{
    HoleReason, NativeHint, NativeInfo, Value, ValueBody, ValueDefinition,
};
pub use type_def::{
    AccessControlled, Constructor, Incompleteness, TypeDefinition,
};
pub use attributes::{
    SourceLocation, TypeAttributes, ValueAttributes,
    // V4 type aliases (preferred for new code)
    V4Type, V4Field, V4Pattern, V4Value, V4ValueDefinition, V4ValueBody,
    V4TypeDefinition, V4Constructor, V4AccessControlledTypeDef,
    // Classic type aliases (for V1-V3 compatibility)
    ClassicAttrs, ClassicType, ClassicField, ClassicPattern, ClassicValue,
    ClassicValueDefinition, ClassicTypeDefinition,
};

// Re-export classic types for backward compatibility (temporary)
pub use classic::*;

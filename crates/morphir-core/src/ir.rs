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

// Legacy support
pub mod classic;

// V4 is the primary format
pub mod v4;

// Re-export serde_tagged and type_def from v4 for backward compatibility
pub use v4::serde_tagged;
pub use v4::type_def;

// Re-exports for V4 types (primary)
pub use v4::{
    // Access control
    Access,
    AccessControlled,
    // Core expression types
    Field,
    Literal,
    Pattern,
    SourceLocation,
    Type,
    TypeAttributes,
    TypeExpr,
    Value,
    ValueAttributes,
    ValueExpr,
    // Value expression types (from value module)
    InputType,
    LetBinding,
    NativeInfo,
    PatternCase,
    RecordFieldEntry,
    ValueExprBody as ValueBody,
    ValueExprDefinition,
    ValueHoleReason,
    ValueNativeHint,
    // Distribution types
    ApplicationContent,
    Dependencies,
    Distribution,
    EntryPoint,
    EntryPointKind,
    EntryPoints,
    LibraryContent,
    SpecsContent,
    // Module types
    ModuleDefinition,
    ModuleSpecification,
    // Package types
    PackageDefinition,
    PackageSpecification,
    // Type definition types
    ConstructorArg,
    ConstructorArgSpec,
    ConstructorDefinition,
    ConstructorSpecification,
    Incompleteness,
    TypeDefinition as V4TypeDefinition,
    TypeSpecification,
    // Value definition types
    HoleReason,
    InputTypeEntry,
    NativeHint,
    ValueBody as ValueDefBody,
    ValueDefinition,
    ValueSpecification,
    // Top-level types
    FormatVersion,
    IRFile,
};

// Re-export type_def types (still in parent ir module for now)
pub use type_def::{
    AccessControlledConstructors, AccessControlledTypeDefinition,
    ConstructorArg as TypeDefConstructorArg, ConstructorDefinition as TypeDefConstructorDefinition,
    TypeDefinition as LegacyTypeDefinition, TypeSpecification as LegacyTypeSpecification,
};

// Re-export classic types for backward compatibility (temporary)
pub use classic::*;

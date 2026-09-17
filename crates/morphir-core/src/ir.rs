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

// A genuine decimal, shared by the v4 model and (task 5) the classic model
pub mod decimal;
pub use decimal::{DecimalLiteral, InvalidDecimalLexeme};

// Diagnostics shared by every IR codec
pub mod diagnostic;
pub use diagnostic::{Diagnostic, DiagnosticCode, DiagnosticError, DiagnosticStage, Warning};

// V4 is the primary format
pub mod v4;

// The JSON storage profile's canonical writer
pub mod json;

// The native YAML storage profile
pub mod yaml;

// Re-export serde_tagged from v4 for backward compatibility
pub use v4::serde_tagged;

// Re-exports for V4 types (primary)
pub use v4::{
    // Access control
    Access,
    AccessControlled,
    // Distribution types
    ApplicationContent,
    // Type definition types
    ConstructorArg,
    ConstructorArgSpec,
    ConstructorDefinition,
    ConstructorSpecification,
    Dependencies,
    Distribution,
    EntryPoint,
    EntryPointKind,
    EntryPoints,
    // Core expression types
    Field,
    // Top-level types
    FormatVersion,
    // Value definition types
    HoleReason,
    IRFile,
    Incompleteness,
    // Value expression types (from value module)
    InputType,
    LetBinding,
    LibraryContent,
    Literal,
    // Module types
    ModuleDefinition,
    ModuleSpecification,
    NativeHint,
    NativeInfo,
    // Package types
    PackageDefinition,
    PackageSpecification,
    Pattern,
    PatternCase,
    RecordFieldEntry,
    SourceLocation,
    SpecsContent,
    Type,
    TypeAttributes,
    TypeDefinition,
    TypeExpr,
    TypeSpecification,
    Value,
    ValueAttributes,
    ValueBody as ValueDefBody,
    ValueDefinition,
    ValueExpr,
    ValueExprBody as ValueBody,
    ValueExprDefinition,
    ValueHoleReason,
    ValueNativeHint,
    ValueSpecification,
};

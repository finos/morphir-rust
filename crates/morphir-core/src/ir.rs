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

// Document-tree layout: the storage profile, path grammar and file stems.
pub mod layout;

// The Ion document-tree profile. A tree file is one JSON value.
pub mod ion;

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
    DefinitionDependencies,
    Dependencies,
    Distribution,
    // The four files a document tree is made of
    DistributionKind,
    DistributionManifestFile,
    EntryPoint,
    EntryPointKind,
    EntryPoints,
    ExpectedEntries,
    FILE_STEM_PATTERN,
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
    MIN_PATH_BUDGET,
    // Module types
    ModuleDefinition,
    ModuleEntries,
    ModuleManifestFile,
    ModuleSpecification,
    NativeHint,
    NativeInfo,
    NodeFileBody,
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
    TypeDefinitionFile,
    TypeExpr,
    TypeSpecification,
    Value,
    ValueAttributes,
    ValueBody as ValueDefBody,
    ValueDefinition,
    ValueDefinitionFile,
    ValueExpr,
    ValueExprBody as ValueBody,
    ValueExprDefinition,
    ValueHoleReason,
    ValueNativeHint,
    ValueSpecification,
};

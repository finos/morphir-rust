//! Morphir IR V4
//!
//! This module defines the structure for Morphir IR Version 4.
//! It supports the Document Tree structure and Canonical Strings.
//!
//! V4 uses object wrapper format for enums and keyed objects (IndexMap) for
//! dictionaries rather than arrays of tuples.

use schemars::JsonSchema;
use serde::Deserializer;
use serde::{Deserialize, Serialize};

use crate::format_version::{
    CanonicalSpelling, NormalizedFormatVersion, ScalarValue, SupportTable,
};

// Submodules - Core IR types
pub mod access;
pub mod annotation;
pub mod attributes;
pub mod distribution;
pub mod legacy;
pub mod literal;
pub mod module;
pub mod package;
pub mod pattern;
pub mod serde_document;
pub mod serde_tagged;
pub mod serde_v4;
pub mod type_def;
pub mod types;
pub mod value;

// Re-export naming types - Name now serializes as V4 canonical format (kebab-case string)
pub use crate::naming::ModuleName;
pub use crate::naming::Name;
pub use crate::naming::PackageName;
pub use crate::naming::Path;

// Re-export access control
pub use access::{Access, AccessControlled};

// Re-export annotations, which specifications carry and definitions do not
pub use annotation::{Annotation, AnnotationArgument};

// Re-export core expression types
pub use crate::ir::decimal::{DecimalLiteral, InvalidDecimalLexeme};
pub use attributes::{SourceLocation, TypeAttributes, TypeExpr, ValueAttributes, ValueExpr};
pub use legacy::{SpellingMode, accept_member, take_warnings, with_spelling_mode};
pub use literal::{FloatLiteral, InvalidFloatLexeme, Literal};
pub use pattern::Pattern;
pub use serde_v4::{TypeEncoding, with_type_encoding};
pub use types::{Field, Type};
pub use value::{
    HoleReason as ValueHoleReason, InputType, LetBinding, NativeHint as ValueNativeHint,
    NativeInfo, PatternCase, RecordFieldEntry, Value, ValueBody as ValueExprBody,
    ValueDefinition as ValueExprDefinition,
};

// Re-export distribution types
pub use distribution::{
    ApplicationContent, DefinitionDependencies, Dependencies, Distribution, EntryPoint,
    EntryPointKind, EntryPoints, LibraryContent, SpecsContent,
};

// Re-export module types
pub use module::{Documentation, Documented, ModuleDefinition, ModuleSpecification};

// Re-export package types
pub use package::{PackageDefinition, PackageSpecification};

// Re-export type definition types
pub use types::{
    ConstructorArg, ConstructorArgSpec, ConstructorDefinition, ConstructorSpecification,
    Incompleteness, TypeDefinition, TypeSpecification,
};

// Re-export value definition types
pub use value::{
    ExternalBinding, HoleReason, NativeHint, ValueBody, ValueDefinition, ValueSpecification,
};

// Re-export legacy type_def types for backward compatibility
pub use type_def::{
    AccessControlledConstructors, AccessControlledTypeDefinition,
    ConstructorArg as TypeDefConstructorArg, ConstructorDefinition as TypeDefConstructorDefinition,
    TypeDefinition as LegacyTypeDefinition, TypeSpecification as LegacyTypeSpecification,
};

/// Top-level IR file structure.
///
/// `formatVersion` comes first and `distribution` second; a document that writes them the other
/// way round is the same document. `$meta` is reserved for the files of a document tree, not for
/// a single document, so it is unknown here (distributions-0009).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IRFile {
    pub format_version: FormatVersion,
    pub distribution: Distribution,
}

impl<'de> Deserialize<'de> for IRFile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        serde_document::deserialize_with(deserializer, serde_document::decode_ir_file)
    }
}

/// Decodes a version 4 document from an already-parsed value tree, with its warnings.
///
/// This is the entry a reader uses when it produced the value tree itself — the YAML profile
/// reader, say — rather than handing a document to serde. It decodes exactly what
/// `Deserialize for IRFile` decodes, and collects the `legacy_spelling` warnings the serde path
/// leaves to its caller's `with_spelling_mode` scope.
pub fn decode_ir_file_with_warnings(
    value: &serde_json::Value,
) -> Result<(IRFile, Vec<crate::ir::Warning>), crate::ir::DiagnosticError> {
    let (decoded, warnings) = with_spelling_mode(SpellingMode::Current, || {
        serde_document::decode_ir_file(value, "")
    });
    decoded
        .map(|file| (file, warnings))
        .map_err(crate::ir::DiagnosticError)
}

/// Format version - accepts both string "4.0.0" and integer 4 using the shared contract.
#[derive(Debug, Clone, PartialEq, JsonSchema)]
pub enum FormatVersion {
    String(String),
    Integer(u32),
}

impl From<CanonicalSpelling> for FormatVersion {
    fn from(value: CanonicalSpelling) -> Self {
        match value {
            CanonicalSpelling::Integer(version) => Self::Integer(version),
            CanonicalSpelling::String(version) => Self::String(version),
        }
    }
}

impl FormatVersion {
    /// Normalize this wire spelling against the reference support table.
    pub fn normalize(
        &self,
    ) -> Result<NormalizedFormatVersion, crate::format_version::FormatVersionDiagnostic> {
        let scalar = match self {
            Self::Integer(version) => ScalarValue::Integer(*version as u64),
            Self::String(version) => ScalarValue::String(version.clone()),
        };
        NormalizedFormatVersion::from_scalar(&scalar, &SupportTable::reference())
    }
}

impl Serialize for FormatVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            FormatVersion::String(s) => serializer.serialize_str(s),
            FormatVersion::Integer(n) => serializer.serialize_u32(*n),
        }
    }
}

impl<'de> Deserialize<'de> for FormatVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        serde_document::deserialize_with(deserializer, serde_document::decode_format_version)
    }
}

impl Default for FormatVersion {
    fn default() -> Self {
        FormatVersion::Integer(4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_version_string() {
        let v = FormatVersion::String("4.0.0".to_string());
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, "\"4.0.0\"");
    }

    #[test]
    fn test_format_version_integer() {
        let v = FormatVersion::Integer(4);
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, "4");
    }
}

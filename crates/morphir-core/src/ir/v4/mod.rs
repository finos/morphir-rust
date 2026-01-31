//! Morphir IR V4
//!
//! This module defines the structure for Morphir IR Version 4.
//! It supports the Document Tree structure and Canonical Strings.
//!
//! V4 uses object wrapper format for enums and keyed objects (IndexMap) for
//! dictionaries rather than arrays of tuples.

use schemars::JsonSchema;
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

// Submodules - Core IR types
pub mod access;
pub mod attributes;
pub mod distribution;
pub mod literal;
pub mod module;
pub mod package;
pub mod pattern;
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

// Re-export core expression types
pub use attributes::{SourceLocation, TypeAttributes, TypeExpr, ValueAttributes, ValueExpr};
pub use literal::Literal;
pub use pattern::Pattern;
pub use types::{Field, Type};
pub use value::{
    HoleReason as ValueHoleReason, InputType, LetBinding, NativeHint as ValueNativeHint,
    NativeInfo, PatternCase, RecordFieldEntry, Value, ValueBody as ValueExprBody,
    ValueDefinition as ValueExprDefinition,
};

// Re-export distribution types
pub use distribution::{
    ApplicationContent, Dependencies, Distribution, EntryPoint, EntryPointKind, EntryPoints,
    LibraryContent, SpecsContent,
};

// Re-export module types
pub use module::{ModuleDefinition, ModuleSpecification};

// Re-export package types
pub use package::{PackageDefinition, PackageSpecification};

// Re-export type definition types
pub use types::{
    ConstructorArg, ConstructorArgSpec, ConstructorDefinition, ConstructorSpecification,
    Incompleteness, TypeDefinition, TypeSpecification,
};

// Re-export value definition types
pub use value::{HoleReason, InputTypeEntry, NativeHint, ValueBody, ValueDefinition, ValueSpecification};

// Re-export legacy type_def types for backward compatibility
pub use type_def::{
    AccessControlledConstructors, AccessControlledTypeDefinition,
    ConstructorArg as TypeDefConstructorArg, ConstructorDefinition as TypeDefConstructorDefinition,
    TypeDefinition as LegacyTypeDefinition, TypeSpecification as LegacyTypeSpecification,
};

/// Top-level IR file structure
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IRFile {
    pub format_version: FormatVersion,
    pub distribution: Distribution,
}

/// Format version - accepts both string "4.0.0" and integer 4
#[derive(Debug, Clone, PartialEq, JsonSchema)]
pub enum FormatVersion {
    String(String),
    Integer(u32),
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
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(s) => Ok(FormatVersion::String(s)),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_u64() {
                    Ok(FormatVersion::Integer(i as u32))
                } else {
                    Err(de::Error::custom(
                        "format version must be a positive integer",
                    ))
                }
            }
            _ => Err(de::Error::custom(
                "format version must be a string or integer",
            )),
        }
    }
}

impl Default for FormatVersion {
    fn default() -> Self {
        FormatVersion::String("4.0.0".to_string())
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

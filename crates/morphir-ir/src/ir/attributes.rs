//! V4 Attributes for Morphir IR.
//!
//! This module defines the attribute types used in V4 format:
//! - `TypeAttributes`: Metadata attached to type nodes
//! - `ValueAttributes`: Metadata attached to value nodes
//!
//! It also provides type aliases for ergonomic V4 usage:
//! - `V4Type`, `V4Pattern`, `V4Value`, etc.

use serde::{Deserialize, Serialize};

use super::pattern::Pattern;
use super::type_def::{AccessControlled, Constructor, TypeDefinition};
use super::type_expr::{Field, Type};
use super::value_expr::{Value, ValueBody, ValueDefinition};

/// Source location information for error messages and tooling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLocation {
    /// Starting line number (1-indexed)
    pub start_line: u32,
    /// Starting column number (1-indexed)
    pub start_column: u32,
    /// Ending line number (1-indexed)
    pub end_line: u32,
    /// Ending column number (1-indexed)
    pub end_column: u32,
}

/// V4 attributes for type expressions.
///
/// Rich metadata attached to type nodes in V4 format, supporting:
/// - Source location tracking for error messages
/// - Type constraints for validation
/// - Tool-specific extensions
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TypeAttributes {
    /// Source location where this type was defined
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceLocation>,

    /// Type constraints (e.g., for constrained type variables)
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub constraints: serde_json::Value,

    /// Tool-specific extensions (IDE hints, optimization notes, etc.)
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub extensions: serde_json::Value,
}

/// V4 attributes for value expressions.
///
/// Rich metadata attached to value nodes in V4 format, supporting:
/// - Source location tracking for error messages
/// - Inferred type information
/// - Tool-specific extensions
///
/// Note: `inferred_type` is stored as JSON until Phase 2 adds serde to Type<A>.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ValueAttributes {
    /// Source location where this value was defined
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceLocation>,

    /// The inferred type of this value (if available)
    /// TODO: Change to Option<Box<Type<TypeAttributes>>> in Phase 2
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub inferred_type: serde_json::Value,

    /// Tool-specific extensions (IDE hints, optimization notes, etc.)
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub extensions: serde_json::Value,
}

// =============================================================================
// V4 Type Aliases - Ergonomic defaults using V4 attributes
// =============================================================================

/// V4 Type expression with TypeAttributes
pub type V4Type = Type<TypeAttributes>;

/// V4 Field in a record type with TypeAttributes
pub type V4Field = Field<TypeAttributes>;

/// V4 Pattern with ValueAttributes
pub type V4Pattern = Pattern<ValueAttributes>;

/// V4 Value expression with TypeAttributes and ValueAttributes
pub type V4Value = Value<TypeAttributes, ValueAttributes>;

/// V4 Value definition with TypeAttributes and ValueAttributes
pub type V4ValueDefinition = ValueDefinition<TypeAttributes, ValueAttributes>;

/// V4 Value body with TypeAttributes and ValueAttributes
pub type V4ValueBody = ValueBody<TypeAttributes, ValueAttributes>;

/// V4 Type definition with TypeAttributes
pub type V4TypeDefinition = TypeDefinition<TypeAttributes>;

/// V4 Constructor with TypeAttributes
pub type V4Constructor = Constructor<TypeAttributes>;

/// V4 Access-controlled type definition
pub type V4AccessControlledTypeDef = AccessControlled<V4TypeDefinition>;

// =============================================================================
// Classic Type Aliases - For backward compatibility with V1-V3
// =============================================================================

/// Classic attributes (empty object {})
pub type ClassicAttrs = serde_json::Value;

/// Classic Type expression with generic JSON attributes
pub type ClassicType = Type<ClassicAttrs>;

/// Classic Field with generic JSON attributes
pub type ClassicField = Field<ClassicAttrs>;

/// Classic Pattern with generic JSON attributes
pub type ClassicPattern = Pattern<ClassicAttrs>;

/// Classic Value with generic JSON attributes
pub type ClassicValue = Value<ClassicAttrs, ClassicAttrs>;

/// Classic Value definition with generic JSON attributes
pub type ClassicValueDefinition = ValueDefinition<ClassicAttrs, ClassicAttrs>;

/// Classic Type definition with generic JSON attributes
pub type ClassicTypeDefinition = TypeDefinition<ClassicAttrs>;

// =============================================================================
// Convenience constructors
// =============================================================================

impl TypeAttributes {
    /// Create empty attributes (equivalent to Classic's `{}`)
    pub fn empty() -> Self {
        TypeAttributes::default()
    }

    /// Create attributes with just a source location
    pub fn with_source(source: SourceLocation) -> Self {
        TypeAttributes {
            source: Some(source),
            constraints: serde_json::Value::Null,
            extensions: serde_json::Value::Null,
        }
    }
}

impl ValueAttributes {
    /// Create empty attributes (equivalent to Classic's `{}`)
    pub fn empty() -> Self {
        ValueAttributes::default()
    }

    /// Create attributes with just a source location
    pub fn with_source(source: SourceLocation) -> Self {
        ValueAttributes {
            source: Some(source),
            inferred_type: serde_json::Value::Null,
            extensions: serde_json::Value::Null,
        }
    }

    /// Create attributes with an inferred type (as JSON)
    /// TODO: Change to accept V4Type in Phase 2
    pub fn with_type_json(inferred_type: serde_json::Value) -> Self {
        ValueAttributes {
            source: None,
            inferred_type,
            extensions: serde_json::Value::Null,
        }
    }
}

impl SourceLocation {
    /// Create a new source location
    pub fn new(start_line: u32, start_column: u32, end_line: u32, end_column: u32) -> Self {
        SourceLocation {
            start_line,
            start_column,
            end_line,
            end_column,
        }
    }

    /// Create a single-point source location
    pub fn point(line: u32, column: u32) -> Self {
        SourceLocation {
            start_line: line,
            start_column: column,
            end_line: line,
            end_column: column,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::naming::Name;

    #[test]
    fn test_type_attributes_default() {
        let attrs = TypeAttributes::default();
        assert!(attrs.source.is_none());
        assert!(attrs.constraints.is_null());
        assert!(attrs.extensions.is_null());
    }

    #[test]
    fn test_type_attributes_with_source() {
        let loc = SourceLocation::new(1, 1, 1, 10);
        let attrs = TypeAttributes::with_source(loc.clone());
        assert_eq!(attrs.source, Some(loc));
    }

    #[test]
    fn test_value_attributes_with_type() {
        let type_json = serde_json::json!(["Unit", {}]);
        let attrs = ValueAttributes::with_type_json(type_json);
        assert!(!attrs.inferred_type.is_null());
    }

    #[test]
    fn test_v4_type_alias() {
        let var: V4Type = Type::variable(TypeAttributes::empty(), Name::from("a"));
        assert!(matches!(var, Type::Variable(_, _)));
    }

    #[test]
    fn test_v4_value_alias() {
        let val: V4Value = Value::unit(ValueAttributes::empty());
        assert!(matches!(val, Value::Unit(_)));
    }

    #[test]
    fn test_source_location_point() {
        let loc = SourceLocation::point(5, 10);
        assert_eq!(loc.start_line, 5);
        assert_eq!(loc.end_line, 5);
        assert_eq!(loc.start_column, 10);
        assert_eq!(loc.end_column, 10);
    }

    #[test]
    fn test_type_attributes_serialization() {
        let attrs = TypeAttributes::with_source(SourceLocation::new(1, 1, 2, 5));
        let json = serde_json::to_string(&attrs).unwrap();
        assert!(json.contains("startLine"));
        assert!(json.contains("endColumn"));

        let parsed: TypeAttributes = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, attrs);
    }

    #[test]
    fn test_empty_attributes_minimal_json() {
        let attrs = TypeAttributes::empty();
        let json = serde_json::to_string(&attrs).unwrap();
        // Empty attributes should serialize to minimal JSON
        assert_eq!(json, "{}");
    }
}

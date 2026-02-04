//! Type expressions and definitions for Morphir IR V4
//!
//! This module defines:
//! - `Type` enum: Type expressions in the Morphir IR
//! - `TypeDefinition` enum: Type definitions (aliases and custom types)
//! - `TypeSpecification` enum: Public API view of types
//!
//! # Examples
//!
//! ```rust,ignore
//! let t: Type = Type::Unit(TypeAttributes::default());
//! ```

use serde::de::{self, Deserializer};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};

use super::access::AccessControlled;
use super::attributes::TypeAttributes;
use super::value::HoleReason;
use crate::naming::{FQName, Name};

// =============================================================================
// Type Expressions
// =============================================================================

/// A type expression with V4 attributes.
///
/// Type expressions form the type system of Morphir IR. Each variant
/// carries `TypeAttributes` which can store metadata like
/// source locations, type constraints, or extensions.
///
/// # Examples
///
/// ```rust,ignore
/// let t: Type = Type::Unit(TypeAttributes::default());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// Type variable (generic type parameter)
    ///
    /// Example: `a` in `List a`
    Variable(TypeAttributes, Name),

    /// Reference to a named type
    ///
    /// Example: `List Int` is `Reference(_, fqname_of_list, [Type::Reference(_, fqname_of_int, [])])`
    Reference(TypeAttributes, FQName, Vec<Type>),

    /// Tuple type (product type with positional elements)
    ///
    /// Example: `(Int, String, Bool)`
    Tuple(TypeAttributes, Vec<Type>),

    /// Record type (product type with named fields)
    ///
    /// Example: `{ name : String, age : Int }`
    Record(TypeAttributes, Vec<Field>),

    /// Extensible record type (record with a row variable)
    ///
    /// Example: `{ a | name : String }` where `a` is the row variable
    ExtensibleRecord(TypeAttributes, Name, Vec<Field>),

    /// Function type (arrow type)
    ///
    /// Example: `Int -> String`
    Function(TypeAttributes, Box<Type>, Box<Type>),

    /// Unit type (empty tuple, void equivalent)
    ///
    /// Example: `()`
    Unit(TypeAttributes),
}

/// A field in a record type.
///
/// Fields have a name and a type.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// The name of the field
    pub name: Name,
    /// The type of the field
    pub tpe: Type,
}

impl Type {
    /// Get the attributes of this type
    pub fn attributes(&self) -> &TypeAttributes {
        match self {
            Type::Variable(a, _) => a,
            Type::Reference(a, _, _) => a,
            Type::Tuple(a, _) => a,
            Type::Record(a, _) => a,
            Type::ExtensibleRecord(a, _, _) => a,
            Type::Function(a, _, _) => a,
            Type::Unit(a) => a,
        }
    }

    /// Create a variable type
    pub fn variable(attrs: TypeAttributes, name: Name) -> Self {
        Type::Variable(attrs, name)
    }

    /// Create a reference type
    pub fn reference(attrs: TypeAttributes, fqname: FQName, type_params: Vec<Type>) -> Self {
        Type::Reference(attrs, fqname, type_params)
    }

    /// Create a tuple type
    pub fn tuple(attrs: TypeAttributes, elements: Vec<Type>) -> Self {
        Type::Tuple(attrs, elements)
    }

    /// Create a record type
    pub fn record(attrs: TypeAttributes, fields: Vec<Field>) -> Self {
        Type::Record(attrs, fields)
    }

    /// Create an extensible record type
    pub fn extensible_record(attrs: TypeAttributes, variable: Name, fields: Vec<Field>) -> Self {
        Type::ExtensibleRecord(attrs, variable, fields)
    }

    /// Create a function type
    pub fn function(attrs: TypeAttributes, arg: Type, result: Type) -> Self {
        Type::Function(attrs, Box::new(arg), Box::new(result))
    }

    /// Create a unit type
    pub fn unit(attrs: TypeAttributes) -> Self {
        Type::Unit(attrs)
    }
}

impl Field {
    /// Create a new field
    pub fn new(name: Name, tpe: Type) -> Self {
        Field { name, tpe }
    }
}

// =============================================================================
// Type Specifications (Public API)
// =============================================================================

/// Type specification (public API view of a type)
// The variant names include "Specification" suffix as per the Morphir specification
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TypeSpecification {
    /// Type alias specification
    TypeAliasSpecification {
        type_params: Vec<Name>,
        #[serde(rename = "typeExp")]
        type_expr: Type,
    },
    /// Opaque type (constructors hidden)
    OpaqueTypeSpecification { type_params: Vec<Name> },
    /// Custom type with public constructors
    CustomTypeSpecification {
        type_params: Vec<Name>,
        constructors: Vec<ConstructorSpecification>,
    },
}

/// Constructor specification
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstructorSpecification {
    pub name: Name,
    pub args: Vec<ConstructorArgSpec>,
}

/// Constructor argument specification
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstructorArgSpec {
    pub name: Name,
    #[serde(rename = "type")]
    pub arg_type: Type,
}

// =============================================================================
// Type Definitions
// =============================================================================

/// Reason for an incomplete type (V4 only)
///
/// Used when a type cannot be fully resolved due to errors or work in progress.
#[derive(Debug, Clone, PartialEq)]
pub enum Incompleteness {
    /// Type has unresolved dependencies or errors
    Hole(HoleReason),
    /// Type is work in progress
    Draft,
}

impl Serialize for Incompleteness {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            Incompleteness::Draft => map.serialize_entry("Draft", &serde_json::json!({}))?,
            Incompleteness::Hole(reason) => map.serialize_entry("Hole", reason)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Incompleteness {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match &value {
            serde_json::Value::Object(map) => {
                if let Some((key, content)) = map.iter().next() {
                    match key.as_str() {
                        "Draft" => Ok(Incompleteness::Draft),
                        "Hole" => {
                            let reason: HoleReason =
                                serde_json::from_value(content.clone()).map_err(de::Error::custom)?;
                            Ok(Incompleteness::Hole(reason))
                        }
                        _ => Err(de::Error::unknown_variant(key, &["Draft", "Hole"])),
                    }
                } else {
                    Err(de::Error::custom("empty object for Incompleteness"))
                }
            }
            // Also accept string format for backward compatibility (Draft only)
            serde_json::Value::String(s) => match s.as_str() {
                "Draft" => Ok(Incompleteness::Draft),
                _ => Err(de::Error::unknown_variant(s, &["Draft"])),
            },
            _ => Err(de::Error::custom(
                "expected object or string for Incompleteness",
            )),
        }
    }
}

/// Type definition - uses wrapper object format
///
/// V4 adds IncompleteTypeDefinition for incremental compilation and error recovery.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeDefinition {
    /// Type alias definition
    ///
    /// Example: `type alias Person = { name : String, age : Int }`
    TypeAliasDefinition {
        type_params: Vec<Name>,
        type_expr: Type,
    },
    /// Custom type (algebraic data type) definition
    ///
    /// Example: `type Maybe a = Just a | Nothing`
    CustomTypeDefinition {
        type_params: Vec<Name>,
        constructors: AccessControlled<Vec<ConstructorDefinition>>,
    },
    /// Incomplete type definition (V4 only)
    ///
    /// Represents types that couldn't be fully resolved.
    /// Used for incremental compilation and error recovery.
    IncompleteTypeDefinition {
        type_params: Vec<Name>,
        incompleteness: Incompleteness,
    },
}

impl Serialize for TypeDefinition {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            TypeDefinition::TypeAliasDefinition {
                type_params,
                type_expr,
            } => {
                map.serialize_entry(
                    "TypeAliasDefinition",
                    &TypeAliasDefContent {
                        type_params: type_params.clone(),
                        type_exp: type_expr.clone(),
                    },
                )?;
            }
            TypeDefinition::CustomTypeDefinition {
                type_params,
                constructors,
            } => {
                map.serialize_entry(
                    "CustomTypeDefinition",
                    &CustomTypeDefContent {
                        type_params: type_params.clone(),
                        constructors: constructors.clone(),
                    },
                )?;
            }
            TypeDefinition::IncompleteTypeDefinition {
                type_params,
                incompleteness,
            } => {
                map.serialize_entry(
                    "IncompleteTypeDefinition",
                    &IncompleteTypeDefContent {
                        type_params: type_params.clone(),
                        incompleteness: incompleteness.clone(),
                    },
                )?;
            }
        }
        map.end()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TypeAliasDefContent {
    type_params: Vec<Name>,
    type_exp: Type,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CustomTypeDefContent {
    type_params: Vec<Name>,
    constructors: AccessControlled<Vec<ConstructorDefinition>>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IncompleteTypeDefContent {
    type_params: Vec<Name>,
    incompleteness: Incompleteness,
}

impl<'de> Deserialize<'de> for TypeDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if let serde_json::Value::Object(map) = &value {
            if let Some(content) = map.get("TypeAliasDefinition") {
                let parsed: TypeAliasDefContent =
                    serde_json::from_value(content.clone()).map_err(de::Error::custom)?;
                return Ok(TypeDefinition::TypeAliasDefinition {
                    type_params: parsed.type_params,
                    type_expr: parsed.type_exp,
                });
            }
            if let Some(content) = map.get("CustomTypeDefinition") {
                let parsed: CustomTypeDefContent =
                    serde_json::from_value(content.clone()).map_err(de::Error::custom)?;
                return Ok(TypeDefinition::CustomTypeDefinition {
                    type_params: parsed.type_params,
                    constructors: parsed.constructors,
                });
            }
            if let Some(content) = map.get("IncompleteTypeDefinition") {
                let parsed: IncompleteTypeDefContent =
                    serde_json::from_value(content.clone()).map_err(de::Error::custom)?;
                return Ok(TypeDefinition::IncompleteTypeDefinition {
                    type_params: parsed.type_params,
                    incompleteness: parsed.incompleteness,
                });
            }
        }
        Err(de::Error::custom(
            "expected TypeAliasDefinition, CustomTypeDefinition, or IncompleteTypeDefinition wrapper",
        ))
    }
}

/// Constructor definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstructorDefinition {
    pub name: Name,
    pub args: Vec<ConstructorArg>,
}

/// Constructor argument
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstructorArg {
    pub name: Name,
    #[serde(rename = "type")]
    pub arg_type: Type,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variable_type() {
        let var: Type = Type::variable(TypeAttributes::default(), Name::from("a"));
        assert!(matches!(var, Type::Variable(_, _)));
    }

    #[test]
    fn test_unit_type() {
        let unit: Type = Type::unit(TypeAttributes::default());
        assert!(matches!(unit, Type::Unit(_)));
    }

    #[test]
    fn test_function_type() {
        let func: Type = Type::function(
            TypeAttributes::default(),
            Type::unit(TypeAttributes::default()),
            Type::unit(TypeAttributes::default()),
        );
        assert!(matches!(func, Type::Function(_, _, _)));
    }
}

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

use serde::Deserializer;
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
/// Type expressions form the type system of Morphir IR. Each variant carries `TypeAttributes`,
/// which can store metadata like source locations, type constraints, or extensions.
///
/// Incompleteness is not one of these variants: a `Hole` is a value expression, and the
/// incompleteness vocabulary a definition carries is not a type expression at all, so a reader
/// that meets `Hole` or `Draft` where a type belongs refuses it as an unknown node.
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
    /// `a` in `List a`, written `"a"`.
    Variable(TypeAttributes, Name),

    /// Reference to a named type, with its type arguments
    ///
    /// `Int` is `"morphir/SDK:basics#int"`; `List a` is
    /// `{ "Reference": ["morphir/SDK:list#list", "a"] }`. A reference with arguments always
    /// carries its wrapper, because a bare array is a Tuple.
    Reference(TypeAttributes, FQName, Vec<Type>),

    /// Tuple type (product type with positional elements)
    ///
    /// `( Int, String )` is
    /// `{ "Tuple": ["morphir/SDK:basics#int", "morphir/SDK:string#string"] }`.
    Tuple(TypeAttributes, Vec<Type>),

    /// Record type (product type with named fields)
    ///
    /// `{ name : String }` is
    /// `{ "Record": { "fields": { "name": "morphir/SDK:string#string" } } }`. The fields are an
    /// object keyed by field name, so `attributes` can sit beside them and the declaration
    /// order is the member order.
    Record(TypeAttributes, Vec<Field>),

    /// Extensible record type (record with a row variable)
    ///
    /// `{ r | email : String }` is
    /// `{ "ExtensibleRecord": { "variable": "r", "fields": { "email": "morphir/SDK:string#string" } } }`.
    ExtensibleRecord(TypeAttributes, Name, Vec<Field>),

    /// Function type (arrow type), from its parameter type to its return type
    ///
    /// `Int -> String` is
    /// `{ "Function": { "parameterType": "morphir/SDK:basics#int", "returnType": "morphir/SDK:string#string" } }`.
    Function(TypeAttributes, Box<Type>, Box<Type>),

    /// Unit type (empty tuple, void equivalent)
    ///
    /// `()` is `{ "Unit": {} }`.
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
// A type expression is held inline rather than boxed: it is the payload a reader reaches for on
// every declaration, and `TypeAttributes` carries two `serde_json::Value` members, which
// `preserve_order` makes wide enough for Clippy to notice the difference between this variant and
// the ones carrying no type expression at all. Boxing would trade that width for an indirection on
// the hot path and change a widely matched public enum.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum TypeSpecification {
    /// Type alias specification
    TypeAliasSpecification {
        type_params: Vec<Name>,
        type_expr: Type,
    },
    /// Opaque type (constructors hidden)
    OpaqueTypeSpecification { type_params: Vec<Name> },
    /// Custom type with public constructors
    CustomTypeSpecification {
        type_params: Vec<Name>,
        constructors: Vec<ConstructorSpecification>,
    },
    /// A type built from another one, with the two conversions between them
    ///
    /// `{ "DerivedTypeSpecification": { "typeParams": [], "baseType": …, "fromBaseType": …,
    /// "toBaseType": … } }`. All four members are required, and the two conversions are FQNames
    /// rather than expressions.
    DerivedTypeSpecification {
        type_params: Vec<Name>,
        base_type: Type,
        from_base_type: FQName,
        to_base_type: FQName,
    },
}

impl Serialize for TypeSpecification {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Alias<'a> {
            type_params: &'a [Name],
            type_exp: &'a Type,
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Opaque<'a> {
            #[serde(skip_serializing_if = "<[Name]>::is_empty")]
            type_params: &'a [Name],
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Custom<'a> {
            type_params: &'a [Name],
            constructors: indexmap::IndexMap<String, Vec<(&'a Name, &'a Type)>>,
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Derived<'a> {
            type_params: &'a [Name],
            base_type: &'a Type,
            from_base_type: String,
            to_base_type: String,
        }

        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            Self::TypeAliasSpecification {
                type_params,
                type_expr,
            } => map.serialize_entry(
                "TypeAliasSpecification",
                &Alias {
                    type_params,
                    type_exp: type_expr,
                },
            )?,
            Self::OpaqueTypeSpecification { type_params } => {
                map.serialize_entry("OpaqueTypeSpecification", &Opaque { type_params })?
            }
            Self::CustomTypeSpecification {
                type_params,
                constructors,
            } => map.serialize_entry(
                "CustomTypeSpecification",
                &Custom {
                    type_params,
                    constructors: constructors
                        .iter()
                        .map(|constructor| {
                            (
                                constructor.name.to_canonical_string(),
                                constructor
                                    .args
                                    .iter()
                                    .map(|argument| (&argument.name, &argument.arg_type))
                                    .collect(),
                            )
                        })
                        .collect(),
                },
            )?,
            Self::DerivedTypeSpecification {
                type_params,
                base_type,
                from_base_type,
                to_base_type,
            } => map.serialize_entry(
                "DerivedTypeSpecification",
                &Derived {
                    type_params,
                    base_type,
                    from_base_type: from_base_type.to_canonical_string(),
                    to_base_type: to_base_type.to_canonical_string(),
                },
            )?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for TypeSpecification {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        super::serde_document::deserialize_with(
            deserializer,
            super::serde_document::decode_type_specification,
        )
    }
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
///
/// A hole says why it is one: `{ "Hole": { "reason": { "Draft": {} } } }`. A draft is
/// deliberately unfinished rather than broken, so it has no reason at all and takes an empty
/// payload: `{ "Draft": {} }`.
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
        #[derive(Serialize)]
        struct HoleContent<'a> {
            reason: &'a HoleReason,
        }

        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            Incompleteness::Draft => map.serialize_entry("Draft", &serde_json::json!({}))?,
            Incompleteness::Hole(reason) => map.serialize_entry("Hole", &HoleContent { reason })?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Incompleteness {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        super::serde_document::deserialize_with(
            deserializer,
            super::serde_document::decode_incompleteness,
        )
    }
}

/// Type definition - uses wrapper object format
///
/// V4 adds IncompleteTypeDefinition for incremental compilation and error recovery.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::enum_variant_names)]
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
        partial_type_expr: Option<Type>,
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
                #[derive(Serialize)]
                #[serde(rename_all = "camelCase")]
                struct Canonical<'a> {
                    type_params: &'a [Name],
                    access: &'a super::Access,
                    constructors: indexmap::IndexMap<String, Vec<(&'a Name, &'a Type)>>,
                }
                map.serialize_entry(
                    "CustomTypeDefinition",
                    &Canonical {
                        type_params,
                        access: &constructors.access,
                        constructors: constructors
                            .value
                            .iter()
                            .map(|constructor| {
                                (
                                    constructor.name.to_canonical_string(),
                                    constructor
                                        .args
                                        .iter()
                                        .map(|argument| (&argument.name, &argument.arg_type))
                                        .collect(),
                                )
                            })
                            .collect(),
                    },
                )?;
            }
            TypeDefinition::IncompleteTypeDefinition {
                type_params,
                incompleteness,
                partial_type_expr,
            } => {
                map.serialize_entry(
                    "IncompleteTypeDefinition",
                    &IncompleteTypeDefContent {
                        type_params: type_params.clone(),
                        incompleteness: incompleteness.clone(),
                        partial_type_exp: partial_type_expr.clone(),
                    },
                )?;
            }
        }
        map.end()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TypeAliasDefContent {
    type_params: Vec<Name>,
    type_exp: Type,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IncompleteTypeDefContent {
    type_params: Vec<Name>,
    incompleteness: Incompleteness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    partial_type_exp: Option<Type>,
}

impl<'de> Deserialize<'de> for TypeDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        super::serde_document::deserialize_with(
            deserializer,
            super::serde_document::decode_type_definition,
        )
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
    fn test_constructor_name_roundtrips_through_canonical_map_key() {
        // An initialism serializes as an uppercase canonical segment ("GC" -> "GC"),
        // so the map-key decode must parse the canonical encoding, not treat the key
        // as a raw word: decoding "GC" as a word would give the distinct name "gc".
        let def = TypeDefinition::CustomTypeDefinition {
            type_params: vec![],
            constructors: AccessControlled {
                access: super::super::Access::Public,
                value: vec![ConstructorDefinition {
                    name: Name::from("GC"),
                    args: vec![],
                }],
            },
        };
        let json = serde_json::to_string(&def).unwrap();
        assert!(json.contains("\"GC\""), "canonical key expected in {json}");
        let back: TypeDefinition = serde_json::from_str(&json).unwrap();
        // Name words lowercase under the canonical encoding, so compare the
        // canonical forms (the serialized shape is the fixed point).
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
        match back {
            TypeDefinition::CustomTypeDefinition { constructors, .. } => {
                assert_eq!(constructors.value[0].name.to_title_case(), "GC");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn test_constructor_spec_name_roundtrips_through_canonical_map_key() {
        let spec = TypeSpecification::CustomTypeSpecification {
            type_params: vec![],
            constructors: vec![ConstructorSpecification {
                name: Name::from("GC"),
                args: vec![],
            }],
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert!(json.contains("\"GC\""), "canonical key expected in {json}");
        let back: TypeSpecification = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
        match back {
            TypeSpecification::CustomTypeSpecification { constructors, .. } => {
                assert_eq!(constructors[0].name.to_title_case(), "GC");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
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

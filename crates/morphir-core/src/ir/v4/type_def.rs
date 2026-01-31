//! Type definition types for Morphir IR V4

use serde::de::{self, Deserializer};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};

use super::access::Access;
use super::types::{ConstructorSpecification, Type};
use crate::naming::Name;

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

/// Access-controlled type definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessControlledTypeDefinition {
    pub access: Access,
    #[serde(flatten)]
    pub value: TypeDefinition,
}

/// Type definition - uses wrapper object format
#[derive(Debug, Clone, PartialEq)]
pub enum TypeDefinition {
    TypeAliasDefinition {
        type_params: Vec<Name>,
        type_expr: Type,
    },
    CustomTypeDefinition {
        type_params: Vec<Name>,
        constructors: AccessControlledConstructors,
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
    constructors: AccessControlledConstructors,
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
        }
        Err(de::Error::custom(
            "expected TypeAliasDefinition or CustomTypeDefinition wrapper",
        ))
    }
}

/// Access-controlled constructors
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessControlledConstructors {
    pub access: Access,
    pub value: Vec<ConstructorDefinition>,
}

/// Constructor definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstructorDefinition {
    pub name: Name,
    pub args: Vec<ConstructorArg>,
}

/// Constructor argument (tuple struct for backward compatibility)
///
/// Note: Serialization is provided by serde_tagged.rs for tuple-style format [name, type]
#[derive(Debug, Clone, PartialEq)]
pub struct ConstructorArg(pub Name, pub Type);

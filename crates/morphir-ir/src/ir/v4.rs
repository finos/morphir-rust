//! Morphir IR V4
//!
//! This module defines the structure for Morphir IR Version 4.
//! It supports the Document Tree structure and Canonical Strings.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Distribution {
    pub format_version: u32,
    pub distribution: DistributionBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum DistributionBody {
    Library(LibraryDistribution),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LibraryDistribution(
    pub LibraryTag,
    pub PackageName,
    pub Dependencies,
    pub PackageDefinition,
);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum LibraryTag {
    Library,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PackageDefinition {
    pub modules: Vec<ModuleDefinitionEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModuleDefinitionEntry(pub ModuleName, pub AccessControlledModuleDefinition);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AccessControlledModuleDefinition {
    pub access: Access,
    pub value: ModuleDefinition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModuleDefinition {
    pub types: Vec<TypeDefinitionEntry>,
    pub values: Vec<ValueDefinitionEntry>,
    pub doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TypeDefinitionEntry(pub Name, pub AccessControlledTypeDefinition);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AccessControlledTypeDefinition {
    pub access: Access,
    pub value: TypeDefinition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum TypeDefinition {
    TypeAlias(TypeAliasDefinition),
    CustomType(CustomTypeDefinition),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TypeAliasDefinition {
    pub type_params: Vec<Name>,
    pub type_exp: Type,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CustomTypeDefinition {
    pub type_params: Vec<Name>,
    pub constructors: AccessControlledConstructors,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AccessControlledConstructors {
    pub access: Access,
    pub value: Constructors,
}

pub type Constructors = Vec<ConstructorDefinition>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConstructorDefinition(pub Name, pub Vec<(Name, Type)>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ValueDefinitionEntry(pub Name, pub AccessControlledValueDefinition);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AccessControlledValueDefinition {
    pub access: Access,
    pub value: ValueDefinition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ValueDefinition {
    pub input_types: Vec<(Name, TypeAttributes, Type)>,
    pub output_type: Type,
    pub body: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum Access {
    Public,
    Private,
}

// Naming types - now using proper newtypes for type safety
pub use crate::naming::PackageName;
pub use crate::naming::ModuleName;
pub use crate::naming::Name;
pub use crate::naming::Path;

// Dependencies specification
pub type Dependencies = Vec<(PackageName, PackageSpecification)>;
pub type PackageSpecification = HashMap<String, serde_json::Value>;

// Type/Value expressions - using serde_json::Value for now
// TODO: Replace with strongly-typed versions from ir::type_expr, ir::value_expr
// when serde integration for tagged arrays is complete
pub type Type = serde_json::Value;
pub type Value = serde_json::Value;
pub type TypeAttributes = serde_json::Value;

pub type Package = PackageDefinition;

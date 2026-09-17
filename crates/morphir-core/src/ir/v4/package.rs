//! Package types for Morphir IR V4
//!
//! This module contains PackageDefinition, PackageSpecification, and related types.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::access::{Access, AccessControlled};
use super::module::{Documented, ModuleDefinition, ModuleSpecification};
use super::types::{
    ConstructorArgSpec, ConstructorSpecification, TypeDefinition, TypeSpecification,
};
use super::value::{ValueDefinition, ValueSpecification};

/// Package specification (for dependencies)
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageSpecification {
    pub modules: IndexMap<String, ModuleSpecification>,
}

impl<'de> Deserialize<'de> for PackageSpecification {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        super::serde_document::deserialize_with(
            deserializer,
            super::serde_document::decode_package_specification,
        )
    }
}

/// Package definition
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageDefinition {
    pub modules: IndexMap<String, AccessControlled<ModuleDefinition>>,
}

impl PackageDefinition {
    /// The public face of this package: the specification a reader of it sees.
    ///
    /// Only public modules, types and values appear, and each loses what a specification does not
    /// carry: a custom type whose constructors are private becomes opaque, as does an incomplete
    /// type definition, since neither has a public shape to publish. A definition carries no
    /// annotations, so the specification derived from one has none.
    ///
    /// This is infallible, so a public value definition with no output type is skipped rather
    /// than refused: a specification states a value's type, and one still being inferred has
    /// nothing to state yet.
    pub fn to_specification(&self) -> PackageSpecification {
        PackageSpecification {
            modules: self
                .modules
                .iter()
                .filter(|(_, controlled)| controlled.access == Access::Public)
                .map(|(name, controlled)| {
                    (name.clone(), module_to_specification(&controlled.value))
                })
                .collect(),
        }
    }
}

fn module_to_specification(definition: &ModuleDefinition) -> ModuleSpecification {
    ModuleSpecification {
        // A definition carries no annotations, so the specification derived from one has none.
        annotations: Vec::new(),
        types: definition
            .types
            .iter()
            .filter(|(_, controlled)| controlled.access == Access::Public)
            .map(|(name, controlled)| {
                let Documented { doc, value } = &controlled.value;
                (
                    name.clone(),
                    Documented::new(doc.clone(), type_to_specification(value)),
                )
            })
            .collect(),
        values: definition
            .values
            .iter()
            .filter(|(_, controlled)| controlled.access == Access::Public)
            .filter_map(|(name, controlled)| {
                let Documented { doc, value } = &controlled.value;
                let specification = value_to_specification(value)?;
                Some((name.clone(), Documented::new(doc.clone(), specification)))
            })
            .collect(),
        doc: definition.doc.clone(),
    }
}

/// The specification a value definition states, or nothing while its output type is unknown.
fn value_to_specification(definition: &ValueDefinition) -> Option<ValueSpecification> {
    Some(ValueSpecification {
        annotations: Vec::new(),
        inputs: definition.input_types.clone(),
        output: definition.output_type.clone()?,
    })
}

fn type_to_specification(definition: &TypeDefinition) -> TypeSpecification {
    match definition {
        TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr,
        } => TypeSpecification::TypeAliasSpecification {
            annotations: Vec::new(),
            type_params: type_params.clone(),
            type_expr: type_expr.clone(),
        },
        TypeDefinition::CustomTypeDefinition {
            type_params,
            constructors,
        } => match constructors.access {
            Access::Public => TypeSpecification::CustomTypeSpecification {
                annotations: Vec::new(),
                type_params: type_params.clone(),
                constructors: constructors
                    .value
                    .iter()
                    .map(|constructor| ConstructorSpecification {
                        name: constructor.name.clone(),
                        args: constructor
                            .args
                            .iter()
                            .map(|argument| ConstructorArgSpec {
                                name: argument.name.clone(),
                                arg_type: argument.arg_type.clone(),
                            })
                            .collect(),
                    })
                    .collect(),
            },
            Access::Private => TypeSpecification::OpaqueTypeSpecification {
                annotations: Vec::new(),
                type_params: type_params.clone(),
            },
        },
        // A type still being written publishes no shape, so it is opaque until it has one.
        TypeDefinition::IncompleteTypeDefinition { type_params, .. } => {
            TypeSpecification::OpaqueTypeSpecification {
                annotations: Vec::new(),
                type_params: type_params.clone(),
            }
        }
    }
}

impl<'de> Deserialize<'de> for PackageDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        super::serde_document::deserialize_with(
            deserializer,
            super::serde_document::decode_package_definition,
        )
    }
}

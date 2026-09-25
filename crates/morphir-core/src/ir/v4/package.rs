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
        super::serde_document::deserialize_standalone_with(
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
        annotations: Vec::new().into(),
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
        annotations: Vec::new().into(),
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
            annotations: Vec::new().into(),
            type_params: type_params.clone(),
            type_expr: type_expr.clone(),
        },
        TypeDefinition::CustomTypeDefinition {
            type_params,
            constructors,
        } => match constructors.access {
            Access::Public => TypeSpecification::CustomTypeSpecification {
                annotations: Vec::new().into(),
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
                annotations: Vec::new().into(),
                type_params: type_params.clone(),
            },
        },
        // A type still being written publishes no shape, so it is opaque until it has one.
        TypeDefinition::IncompleteTypeDefinition { type_params, .. } => {
            TypeSpecification::OpaqueTypeSpecification {
                annotations: Vec::new().into(),
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
        super::serde_document::deserialize_standalone_with(
            deserializer,
            super::serde_document::decode_package_definition,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::attributes::ValueAttributes;
    use super::*;
    use crate::ir::v4::module::Documentation;
    use crate::ir::v4::types::{ConstructorArg, ConstructorDefinition, Incompleteness};
    use crate::ir::v4::value::{Value, ValueBody};
    use crate::naming::Name;

    fn attrs() -> super::super::attributes::TypeAttributes {
        super::super::attributes::TypeAttributes::default()
    }

    fn unit_type() -> super::super::types::Type {
        super::super::types::Type::unit(attrs())
    }

    fn public<T>(value: T) -> AccessControlled<T> {
        AccessControlled {
            access: Access::Public,
            value,
        }
    }

    fn private<T>(value: T) -> AccessControlled<T> {
        AccessControlled {
            access: Access::Private,
            value,
        }
    }

    fn undocumented<T>(value: T) -> Documented<T> {
        Documented::new(None, value)
    }

    fn documented<T>(doc: &str, value: T) -> Documented<T> {
        Documented::new(Some(Documentation::from(doc)), value)
    }

    fn value_definition(output_type: Option<super::super::types::Type>) -> ValueDefinition {
        ValueDefinition {
            input_types: IndexMap::new(),
            output_type,
            body: ValueBody::Expression(Value::Unit(ValueAttributes::default())),
        }
    }

    /// Builds a package with one private module (dropped entirely) and one public module whose
    /// types and values cover every filtering and translation rule `to_specification` documents:
    /// private members are dropped, a custom type with private constructors and an incomplete
    /// type both become opaque, a public alias keeps its `doc`, and a value with no output type
    /// is skipped.
    fn sample_package() -> PackageDefinition {
        let mut public_module_types = IndexMap::new();
        public_module_types.insert(
            "PrivType".to_string(),
            private(undocumented(TypeDefinition::TypeAliasDefinition {
                type_params: vec![],
                type_expr: unit_type(),
            })),
        );
        public_module_types.insert(
            "OpaqueFromPrivateConstructors".to_string(),
            public(undocumented(TypeDefinition::CustomTypeDefinition {
                type_params: vec![],
                constructors: private(vec![ConstructorDefinition {
                    name: Name::from("hidden"),
                    args: vec![ConstructorArg {
                        name: Name::from("value"),
                        arg_type: unit_type(),
                    }],
                }]),
            })),
        );
        public_module_types.insert(
            "Incomplete".to_string(),
            public(undocumented(TypeDefinition::IncompleteTypeDefinition {
                type_params: vec![],
                incompleteness: Incompleteness::Draft,
                partial_type_expr: None,
            })),
        );
        public_module_types.insert(
            "PublicAlias".to_string(),
            public(documented(
                "An alias for unit.",
                TypeDefinition::TypeAliasDefinition {
                    type_params: vec![],
                    type_expr: unit_type(),
                },
            )),
        );

        let mut public_module_values = IndexMap::new();
        public_module_values.insert(
            "privValue".to_string(),
            private(undocumented(value_definition(Some(unit_type())))),
        );
        public_module_values.insert(
            "noOutputYet".to_string(),
            public(undocumented(value_definition(None))),
        );
        public_module_values.insert(
            "publicValue".to_string(),
            public(undocumented(value_definition(Some(unit_type())))),
        );

        let mut modules = IndexMap::new();
        modules.insert(
            "PrivateModule".to_string(),
            private(ModuleDefinition {
                types: IndexMap::new(),
                values: IndexMap::new(),
                doc: None,
            }),
        );
        modules.insert(
            "PublicModule".to_string(),
            public(ModuleDefinition {
                types: public_module_types,
                values: public_module_values,
                doc: None,
            }),
        );

        PackageDefinition { modules }
    }

    #[test]
    fn to_specification_pins_every_filtering_and_translation_rule() {
        let specification = sample_package().to_specification();

        // A private module has no public face at all, so it does not appear.
        assert_eq!(specification.modules.len(), 1);
        let module = specification.modules.get("PublicModule").unwrap();

        // Order is preserved: types and values keep the order they were declared in, and the
        // private type/value is dropped rather than merely hidden.
        assert_eq!(
            module.types.keys().collect::<Vec<_>>(),
            vec!["OpaqueFromPrivateConstructors", "Incomplete", "PublicAlias"]
        );
        assert_eq!(
            module.values.keys().collect::<Vec<_>>(),
            vec!["publicValue"]
        );

        // A custom type whose constructors are private has nothing public to publish, so it
        // becomes opaque.
        match &module
            .types
            .get("OpaqueFromPrivateConstructors")
            .unwrap()
            .value
        {
            TypeSpecification::OpaqueTypeSpecification { annotations, .. } => {
                assert!(annotations.is_empty());
            }
            other => panic!("expected an opaque specification, got {other:?}"),
        }

        // An incomplete type definition has no shape to publish either, so it is also opaque.
        match &module.types.get("Incomplete").unwrap().value {
            TypeSpecification::OpaqueTypeSpecification { .. } => {}
            other => panic!("expected an opaque specification, got {other:?}"),
        }

        // A public alias becomes an alias specification, carrying the doc from the definition.
        let alias = module.types.get("PublicAlias").unwrap();
        assert_eq!(
            alias.doc.as_ref().map(Documentation::text),
            Some("An alias for unit.")
        );
        match &alias.value {
            TypeSpecification::TypeAliasSpecification { annotations, .. } => {
                assert!(annotations.is_empty());
            }
            other => panic!("expected an alias specification, got {other:?}"),
        }

        // A value definition with no output type is skipped rather than refused.
        assert!(!module.values.contains_key("noOutputYet"));
        assert!(!module.values.contains_key("privValue"));

        // A definition carries no annotations, so nothing derived from one has any either.
        assert!(module.annotations.is_empty());
        assert_eq!(
            module.values.get("publicValue").unwrap().value.annotations,
            Vec::new()
        );
    }
}

//! Interpret the finite built-in declaration vocabulary in native V4 facts.
//!
//! This module is pure. Its caller must authenticate the exact provider IR and
//! every context resource before using the returned closure as trusted meaning.

use super::admission::{
    AdmissionError, Interpretation, Interpreter, NodeTargetKind, ObjectDeclaration,
    PredicateClosure, PredicateDeclaration, SubjectRole,
};
use super::{ContextResources, DocumentId, Fact, ObjectTerm};
use crate::ir::v4::{self, Access, Distribution};
use crate::naming::FQName;
use crate::node_address::{NodeIndex, NodeOwner, NodeResolutionError, NodeRoot, NodeUri};
use std::collections::BTreeMap;

const SCHEMA_VOCAB: &str = "morphir://ir/pkg/morphir/metadata?format=4.1.0#/module/schema/value/";

/// A native declaration is malformed or does not name a public provider value.
#[derive(Debug, thiserror::Error)]
pub enum ProviderDeclarationError {
    #[error("native predicate declarations require V4.1 IR")]
    Format,
    #[error("cannot expand provider declaration facts: {0}")]
    Graph(#[from] v4::DocumentGraphError),
    #[error("cannot index provider declarations: {0}")]
    Index(#[from] NodeResolutionError),
    #[error("invalid provider document identity: {0}")]
    Document(String),
    #[error("native predicate declaration must name a public value with an output type")]
    MissingPublicValue,
    #[error("invalid native predicate declaration: {0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Admission(#[from] AdmissionError),
}

impl PredicateClosure {
    /// Read native declaration facts in an already supplied V4 provider.
    ///
    /// This pure operation does not authenticate the provider. A consumer must
    /// obtain the exact IR and context bytes from its fresh verified Library
    /// restore before treating the resulting closure as trusted.
    pub fn from_v4_provider(
        file: &v4::IRFile,
        resources: &ContextResources,
    ) -> Result<Self, ProviderDeclarationError> {
        if !matches!(&file.format_version, v4::FormatVersion::String(version) if version == "4.1.0")
        {
            return Err(ProviderDeclarationError::Format);
        }
        let index = NodeIndex::v4_file(file)?;
        let owner = index.address_for(&NodeRoot::Distribution, &[])?;
        let owner = DocumentId::new(owner.to_string())
            .map_err(|error| ProviderDeclarationError::Document(error.to_string()))?;
        // Only the schema vocabulary is interpreted here. Other @json facts
        // may need their own declaration and must not create a bootstrap cycle.
        let graph = v4::expand_v4_single_file_graph(file, &owner, resources, |predicate| {
            Some(predicate.clone())
        })?;
        let mut groups: BTreeMap<String, Vec<&Fact>> = BTreeMap::new();
        for fact in graph.facts() {
            if fact.predicate().to_string().starts_with(SCHEMA_VOCAB) {
                groups
                    .entry(fact.subject().to_string())
                    .or_default()
                    .push(fact);
            }
        }
        let declarations = groups
            .into_values()
            .map(|facts| declaration(file, &index, &facts))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::new(declarations)?)
    }
}

fn declaration(
    file: &v4::IRFile,
    index: &NodeIndex,
    facts: &[&Fact],
) -> Result<PredicateDeclaration, ProviderDeclarationError> {
    let uri = facts[0].subject();
    let NodeRoot::Value {
        owner: NodeOwner::OwnPackage,
        module,
        name,
    } = uri.root()
    else {
        return Err(ProviderDeclarationError::MissingPublicValue);
    };
    if !uri.steps().is_empty() || index.address_for(uri.root(), &[])? != *uri {
        return Err(ProviderDeclarationError::MissingPublicValue);
    }
    let module_name = module.to_canonical_string();
    let value_name = name.to_canonical_string();
    let output = match &file.distribution {
        Distribution::Library(library) => library
            .def
            .modules
            .get(&module_name)
            .filter(|item| item.access == Access::Public)
            .and_then(|item| item.value.values.get(&value_name))
            .filter(|item| item.access == Access::Public)
            .and_then(|item| item.value.value.output_type.as_ref()),
        Distribution::Specs(specs) => specs
            .spec
            .modules
            .get(&module_name)
            .and_then(|item| item.values.get(&value_name))
            .map(|item| &item.value.output),
        Distribution::Application(_) => None,
    }
    .ok_or(ProviderDeclarationError::MissingPublicValue)?;
    let mut roles = Vec::new();
    let mut object_form = None;
    let mut node_target_kind = None;
    let mut interpreter = None;
    for fact in facts {
        let predicate = fact.predicate().to_string();
        let term = predicate
            .strip_prefix(SCHEMA_VOCAB)
            .ok_or(ProviderDeclarationError::Invalid("schema predicate"))?;
        let ObjectTerm::Value(value) = fact.object() else {
            return Err(ProviderDeclarationError::Invalid(
                "schema object must be a literal",
            ));
        };
        let value = value
            .value()
            .as_str()
            .ok_or(ProviderDeclarationError::Invalid(
                "schema object must be a string",
            ))?;
        match term {
            "subject-role" => roles.push(subject_role(value)?),
            "object-form" => set_once(&mut object_form, value)?,
            "node-target-kind" => set_once(&mut node_target_kind, value)?,
            "interpreter" => set_once(&mut interpreter, value)?,
            _ => return Err(ProviderDeclarationError::Invalid("unknown schema term")),
        }
    }
    let object =
        match object_form.ok_or(ProviderDeclarationError::Invalid("missing object form"))? {
            "data" => ObjectDeclaration::data(output.clone()),
            "json" => ObjectDeclaration::json(json_type_uri(file, index, output)?, output.clone()),
            "node" => {
                let sdk_string = FQName::from_canonical_string("morphir/SDK:string#string")
                    .expect("built-in String name is valid");
                if !matches!(output, v4::Type::Reference(_, name, arguments)
                    if name == &sdk_string && arguments.is_empty())
                {
                    return Err(ProviderDeclarationError::Invalid(
                        "node reference output must be String",
                    ));
                }
                let kind = match node_target_kind.ok_or(ProviderDeclarationError::Invalid(
                    "missing node target kind",
                ))? {
                    "Type" => NodeTargetKind::Type,
                    "Value" => NodeTargetKind::Value,
                    "Module" => NodeTargetKind::Module,
                    "Package" => NodeTargetKind::Package,
                    _ => {
                        return Err(ProviderDeclarationError::Invalid(
                            "unknown node target kind",
                        ));
                    }
                };
                ObjectDeclaration::node(kind)
            }
            _ => return Err(ProviderDeclarationError::Invalid("unsupported object form")),
        };
    if object_form != Some("node") && node_target_kind.is_some() {
        return Err(ProviderDeclarationError::Invalid(
            "node target kind requires node object form",
        ));
    }
    let interpretation = match interpreter
        .ok_or(ProviderDeclarationError::Invalid("missing interpretation"))?
    {
        "descriptive" => Interpretation::Descriptive,
        "target-name-language-ids" => Interpretation::Required(Interpreter::TargetNameLanguageIds),
        _ => return Err(ProviderDeclarationError::Invalid("unsupported interpreter")),
    };
    Ok(PredicateDeclaration::value(
        uri.clone(),
        object,
        roles,
        interpretation,
    ))
}

fn json_type_uri(
    file: &v4::IRFile,
    index: &NodeIndex,
    output: &v4::Type,
) -> Result<NodeUri, ProviderDeclarationError> {
    let v4::Type::Reference(_, name, arguments) = output else {
        return Err(ProviderDeclarationError::Invalid(
            "JSON output must name a type",
        ));
    };
    if !arguments.is_empty() || &name.package_path != file.distribution.package_name().as_path() {
        return Err(ProviderDeclarationError::Invalid(
            "JSON output must name an unparameterized type in this provider",
        ));
    }
    let module_name = name.module_path.to_canonical_string();
    let type_name = name.local_name.to_canonical_string();
    let public = match &file.distribution {
        Distribution::Library(library) => library
            .def
            .modules
            .get(&module_name)
            .filter(|item| item.access == Access::Public)
            .and_then(|item| item.value.types.get(&type_name))
            .is_some_and(|item| item.access == Access::Public),
        Distribution::Specs(specs) => specs
            .spec
            .modules
            .get(&module_name)
            .is_some_and(|item| item.types.contains_key(&type_name)),
        Distribution::Application(_) => false,
    };
    if !public {
        return Err(ProviderDeclarationError::Invalid(
            "JSON type must be public",
        ));
    }
    Ok(index.address_for(
        &NodeRoot::Type {
            owner: NodeOwner::OwnPackage,
            module: name.module_path.clone(),
            name: name.local_name.clone(),
        },
        &[],
    )?)
}

fn set_once<'a>(
    slot: &mut Option<&'a str>,
    value: &'a str,
) -> Result<(), ProviderDeclarationError> {
    if slot.replace(value).is_some() {
        return Err(ProviderDeclarationError::Invalid(
            "duplicate schema property",
        ));
    }
    Ok(())
}

fn subject_role(value: &str) -> Result<SubjectRole, ProviderDeclarationError> {
    match value {
        "TypeSpecification" => Ok(SubjectRole::TypeSpecification),
        "TypeDefinition" => Ok(SubjectRole::TypeDefinition),
        "ValueSpecification" => Ok(SubjectRole::ValueSpecification),
        "ValueDefinition" => Ok(SubjectRole::ValueDefinition),
        "TypeExpression" => Ok(SubjectRole::TypeExpression),
        "ValueExpression" => Ok(SubjectRole::ValueExpression),
        "Pattern" => Ok(SubjectRole::Pattern),
        "Module" => Ok(SubjectRole::Module),
        "Package" => Ok(SubjectRole::Package),
        _ => Err(ProviderDeclarationError::Invalid("unknown subject role")),
    }
}

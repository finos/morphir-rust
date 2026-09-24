//! Shape-aware conversion of Elm V3 decoration keys.

use super::{NodeIndex, NodeOwner, NodeRoot, NodeStep, NodeUri};
use crate::ir::classic;
use crate::naming::{Name, Path};

/// Why a V3 sidecar key cannot be migrated to a semantic URI.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LegacyNodeIdError {
    #[error("invalid V3 NodeID spelling: {0}")]
    InvalidSpelling(String),
    #[error("V3 NodeID selects a different package")]
    PackageMismatch,
    #[error("V3 NodeID target does not exist")]
    StaleTarget,
    #[error("V3 NodeID step has no reviewed semantic mapping")]
    MappingNotSpecified,
}

enum LegacyStep {
    Name(Name),
    Index(usize),
}

/// Convert a key only after traversing its actual V3 node shapes. The index
/// supplies the canonical guard and confirms that the resulting target exists.
/// A sidecar migrator should convert *all* entries before replacing its file.
pub fn convert_v3_node_id(
    distribution: &classic::Distribution,
    index: &NodeIndex,
    key: &str,
) -> Result<NodeUri, LegacyNodeIdError> {
    if distribution.format_version != 3 {
        return Err(LegacyNodeIdError::InvalidSpelling(key.to_owned()));
    }
    // ChildByName/ChildByIndex steps follow `#` and are themselves separated
    // by `:`. Only the first two separators divide package, module and ID.
    let parts = key.splitn(3, ':').collect::<Vec<_>>();
    if !matches!(parts.len(), 2 | 3) || parts.iter().any(|part| part.is_empty()) {
        return Err(LegacyNodeIdError::InvalidSpelling(key.to_owned()));
    }
    let package = legacy_path(parts[0])?;
    let module = legacy_path(parts[1])?;
    // A v3 Specs distribution (IR 3.1.0) holds no definitions for a NodeID to select.
    let classic::DistributionBody::Library(actual_package, _, definition) =
        &distribution.distribution
    else {
        return Err(LegacyNodeIdError::MappingNotSpecified);
    };
    if package != convert_path(actual_package)? {
        return Err(LegacyNodeIdError::PackageMismatch);
    }
    let owner = NodeOwner::OwnPackage;
    if parts.len() == 2 {
        return index
            .address_for(&NodeRoot::Module { owner, module }, &[])
            .map_err(|_| LegacyNodeIdError::StaleTarget);
    }
    let (definition_name, path) = parts[2].split_once('#').unwrap_or((parts[2], ""));
    if path.contains('#') {
        return Err(LegacyNodeIdError::InvalidSpelling(key.to_owned()));
    }
    let (name, kind) = if let Some(name) = definition_name
        .strip_suffix(".type")
        .or_else(|| definition_name.strip_suffix("/type"))
    {
        (legacy_name(name)?, "type")
    } else if let Some(name) = definition_name
        .strip_suffix(".value")
        .or_else(|| definition_name.strip_suffix("/value"))
    {
        (legacy_name(name)?, "value")
    } else {
        return Err(LegacyNodeIdError::InvalidSpelling(key.to_owned()));
    };
    let legacy_steps = parse_steps(path)?;
    let module_definition = definition
        .modules
        .iter()
        .find(|entry| convert_path(&entry.path).ok().as_ref() == Some(&module))
        .ok_or(LegacyNodeIdError::StaleTarget)?;
    let mut semantic = Vec::new();
    let root = if kind == "type" {
        let (_, controlled) = module_definition
            .definition
            .value
            .types
            .iter()
            .find(|(candidate, _)| convert_name(candidate).ok().as_ref() == Some(&name))
            .ok_or(LegacyNodeIdError::StaleTarget)?;
        let ty = match &controlled.value.value {
            classic::TypeDefinition::Alias(_, ty) => {
                semantic.push(NodeStep::TypeExpression);
                ty
            }
            classic::TypeDefinition::Custom(_, constructors) => {
                let [constructor] = constructors.value.as_slice() else {
                    return Err(LegacyNodeIdError::StaleTarget);
                };
                let [(.., ty)] = constructor.args.as_slice() else {
                    return Err(LegacyNodeIdError::StaleTarget);
                };
                if convert_name(&constructor.name)? != name {
                    return Err(LegacyNodeIdError::StaleTarget);
                }
                semantic.push(NodeStep::CustomConstructor(name.clone()));
                semantic.push(NodeStep::ConstructorArgument(0));
                ty
            }
        };
        map_type_steps(ty, &legacy_steps, &mut semantic)?;
        NodeRoot::Type {
            owner,
            module,
            name,
        }
    } else {
        let (_, controlled) = module_definition
            .definition
            .value
            .values
            .iter()
            .find(|(candidate, _)| convert_name(candidate).ok().as_ref() == Some(&name))
            .ok_or(LegacyNodeIdError::StaleTarget)?;
        semantic.push(NodeStep::Body);
        map_value_steps(&controlled.value.value.body, &legacy_steps, &mut semantic)?;
        NodeRoot::Value {
            owner,
            module,
            name,
        }
    };
    index
        .address_for(&root, &semantic)
        .map_err(|_| LegacyNodeIdError::StaleTarget)
}

fn parse_steps(text: &str) -> Result<Vec<LegacyStep>, LegacyNodeIdError> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    text.split(':')
        .map(|part| {
            if part.is_empty() {
                return Err(LegacyNodeIdError::InvalidSpelling(text.to_owned()));
            }
            if part.bytes().all(|byte| byte.is_ascii_digit()) {
                if part.len() > 1 && part.starts_with('0') {
                    return Err(LegacyNodeIdError::InvalidSpelling(text.to_owned()));
                }
                return part
                    .parse()
                    .map(LegacyStep::Index)
                    .map_err(|_| LegacyNodeIdError::InvalidSpelling(text.to_owned()));
            }
            legacy_name(part).map(LegacyStep::Name)
        })
        .collect()
}

fn legacy_path(text: &str) -> Result<Path, LegacyNodeIdError> {
    if text.is_empty() {
        return Err(LegacyNodeIdError::InvalidSpelling(text.to_owned()));
    }
    Ok(Path {
        segments: text
            .split('.')
            .map(legacy_name)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn legacy_name(text: &str) -> Result<Name, LegacyNodeIdError> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(LegacyNodeIdError::InvalidSpelling(text.to_owned()));
    }
    convert_name(&classic::Name::from_str(text))
}

fn convert_name(name: &classic::Name) -> Result<Name, LegacyNodeIdError> {
    let words = name
        .words
        .iter()
        .map(|word| crate::naming::resolve(*word).to_string())
        .collect::<Vec<_>>();
    let converted = Name::from_words(words);
    Name::from_canonical_string(&converted.to_canonical_string())
        .map_err(|_| LegacyNodeIdError::InvalidSpelling(name.to_string()))
}

fn convert_path(path: &classic::Path) -> Result<Path, LegacyNodeIdError> {
    Ok(Path {
        segments: path
            .segments
            .iter()
            .map(convert_name)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn map_type_steps(
    ty: &classic::Type<classic::Attrs>,
    steps: &[LegacyStep],
    output: &mut Vec<NodeStep>,
) -> Result<(), LegacyNodeIdError> {
    let Some((first, rest)) = steps.split_first() else {
        return Ok(());
    };
    let (step, child) = match (ty, first) {
        (classic::Type::Record(_, fields), LegacyStep::Name(name)) => {
            let field = fields
                .iter()
                .find(|field| convert_name(&field.name).ok().as_ref() == Some(name))
                .ok_or(LegacyNodeIdError::StaleTarget)?;
            (NodeStep::RecordField(name.clone()), &field.ty)
        }
        (classic::Type::ExtensibleRecord(_, _, fields), LegacyStep::Name(name)) => {
            let field = fields
                .iter()
                .find(|field| convert_name(&field.name).ok().as_ref() == Some(name))
                .ok_or(LegacyNodeIdError::StaleTarget)?;
            (NodeStep::ExtensibleRecordField(name.clone()), &field.ty)
        }
        (classic::Type::Record(_, fields), LegacyStep::Index(index)) => {
            let field = fields.get(*index).ok_or(LegacyNodeIdError::StaleTarget)?;
            (NodeStep::RecordField(convert_name(&field.name)?), &field.ty)
        }
        (classic::Type::ExtensibleRecord(_, _, fields), LegacyStep::Index(index)) => {
            let field = fields.get(*index).ok_or(LegacyNodeIdError::StaleTarget)?;
            (
                NodeStep::ExtensibleRecordField(convert_name(&field.name)?),
                &field.ty,
            )
        }
        (classic::Type::Tuple(_, elements), LegacyStep::Index(index)) => (
            NodeStep::TupleElement(*index),
            elements.get(*index).ok_or(LegacyNodeIdError::StaleTarget)?,
        ),
        (classic::Type::Reference(_, _, arguments), LegacyStep::Index(index)) => (
            NodeStep::ReferenceArgument(*index),
            arguments
                .get(*index)
                .ok_or(LegacyNodeIdError::StaleTarget)?,
        ),
        (classic::Type::Function(_, parameter, _), LegacyStep::Index(0)) => {
            (NodeStep::TypeFunctionParameter, parameter.as_ref())
        }
        (classic::Type::Function(_, _, result), LegacyStep::Index(1)) => {
            (NodeStep::TypeFunctionResult, result.as_ref())
        }
        _ => return Err(LegacyNodeIdError::StaleTarget),
    };
    output.push(step);
    map_type_steps(child, rest, output)
}

fn map_value_steps(
    value: &classic::Value<classic::Attrs, classic::Type<classic::Attrs>>,
    steps: &[LegacyStep],
    output: &mut Vec<NodeStep>,
) -> Result<(), LegacyNodeIdError> {
    let Some((first, rest)) = steps.split_first() else {
        return Ok(());
    };
    let (step, child) = match (value, first) {
        (classic::Value::Apply(_, function, _), LegacyStep::Index(0)) => {
            (NodeStep::ApplyFunction, function.as_ref())
        }
        (classic::Value::Apply(_, _, argument), LegacyStep::Index(1)) => {
            (NodeStep::ApplyArgument, argument.as_ref())
        }
        (classic::Value::Field(_, subject, _), LegacyStep::Index(0)) => {
            (NodeStep::FieldSubject, subject.as_ref())
        }
        (classic::Value::Tuple(_, children), LegacyStep::Index(index)) => (
            NodeStep::TupleElement(*index),
            children.get(*index).ok_or(LegacyNodeIdError::StaleTarget)?,
        ),
        (classic::Value::List(_, children), LegacyStep::Index(index)) => (
            NodeStep::ListElement(*index),
            children.get(*index).ok_or(LegacyNodeIdError::StaleTarget)?,
        ),
        (classic::Value::Record(_, fields), LegacyStep::Name(name)) => {
            let (_, child) = fields
                .iter()
                .find(|(candidate, _)| convert_name(candidate).ok().as_ref() == Some(name))
                .ok_or(LegacyNodeIdError::StaleTarget)?;
            (NodeStep::RecordField(name.clone()), child)
        }
        (classic::Value::IfThenElse(_, condition, _, _), LegacyStep::Index(0)) => {
            (NodeStep::IfCondition, condition.as_ref())
        }
        (classic::Value::IfThenElse(_, _, then_value, _), LegacyStep::Index(1)) => {
            (NodeStep::IfThen, then_value.as_ref())
        }
        (classic::Value::IfThenElse(_, _, _, else_value), LegacyStep::Index(2)) => {
            (NodeStep::IfElse, else_value.as_ref())
        }
        (classic::Value::Lambda(_, _, body), LegacyStep::Index(1)) => {
            (NodeStep::LambdaBody, body.as_ref())
        }
        (classic::Value::Destructure(_, _, subject, _), LegacyStep::Index(1)) => {
            (NodeStep::DestructureValue, subject.as_ref())
        }
        (classic::Value::Destructure(_, _, _, body), LegacyStep::Index(2)) => {
            (NodeStep::DestructureBody, body.as_ref())
        }
        (classic::Value::PatternMatch(_, subject, _), LegacyStep::Index(0)) => {
            (NodeStep::PatternMatchSubject, subject.as_ref())
        }
        (classic::Value::Update(_, subject, _), LegacyStep::Index(0)) => {
            (NodeStep::UpdateSubject, subject.as_ref())
        }
        (classic::Value::Update(_, _, fields), LegacyStep::Index(1)) => {
            let [LegacyStep::Name(name), tail @ ..] = rest else {
                return Err(LegacyNodeIdError::StaleTarget);
            };
            let (_, child) = fields
                .iter()
                .find(|(candidate, _)| convert_name(candidate).ok().as_ref() == Some(name))
                .ok_or(LegacyNodeIdError::StaleTarget)?;
            output.push(NodeStep::UpdateField(name.clone()));
            return map_value_steps(child, tail, output);
        }
        _ => return Err(LegacyNodeIdError::MappingNotSpecified),
    };
    output.push(step);
    map_value_steps(child, rest, output)
}

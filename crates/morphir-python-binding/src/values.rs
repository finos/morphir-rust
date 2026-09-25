//! Validation shared by Python function compilation and generation.

use crate::{Outcome, error};
use morphir_core::{ir::v4::*, naming::FQName};
use std::collections::{BTreeMap, BTreeSet};

mod check;
pub(crate) use check::annotate_function;
pub(crate) type Signatures = BTreeMap<String, ValueSpecification>;

pub(crate) fn signatures(library: &LibraryContent) -> Outcome<Signatures> {
    library
        .def
        .modules
        .iter()
        .flat_map(|(module, definition)| {
            definition.value.values.iter().map(move |(name, entry)| {
                let definition = &entry.value.value;
                Ok((
                    format!(
                        "{}:{module}#{name}",
                        library.package_name.to_canonical_string()
                    ),
                    specification(definition)?,
                ))
            })
        })
        .collect()
}

pub(crate) fn specification(definition: &ValueDefinition) -> Outcome<ValueSpecification> {
    Ok(ValueSpecification {
        annotations: Vec::new().into(),
        inputs: definition.input_types.clone(),
        output: definition
            .output_type
            .clone()
            .ok_or_else(|| unsupported("Function return type is required"))?,
    })
}

pub(crate) fn function_type(input: Type, output: Type) -> Type {
    Type::Function(Default::default(), Box::new(input), Box::new(output))
}

pub(crate) fn signature_type(signature: &ValueSpecification) -> Type {
    signature
        .inputs
        .values()
        .rev()
        .fold(signature.output.clone(), |output, input| {
            function_type(input.clone(), output)
        })
}

/// Tuple aliases indexed by their canonical fully qualified names.
pub(crate) type TupleAliases = BTreeMap<String, Type>;

pub(crate) fn resolve_aliases(tpe: &Type, aliases: &TupleAliases) -> Outcome<Type> {
    fn resolve(
        tpe: &Type,
        aliases: &TupleAliases,
        visiting: &mut BTreeSet<String>,
    ) -> Outcome<Type> {
        match tpe {
            Type::Reference(_, name, args) if args.is_empty() => {
                let key = name.to_canonical_string();
                if let Some(alias) = aliases.get(&key) {
                    if !visiting.insert(key.clone()) {
                        return Err(unsupported("Recursive tuple aliases are not supported"));
                    }
                    let result = resolve(alias, aliases, visiting);
                    visiting.remove(&key);
                    result
                } else {
                    Ok(tpe.clone())
                }
            }
            Type::Tuple(attrs, elements) => Ok(Type::Tuple(
                attrs.clone(),
                elements
                    .iter()
                    .map(|element| resolve(element, aliases, visiting))
                    .collect::<Outcome<_>>()?,
            )),
            Type::Function(attrs, input, output) => Ok(Type::Function(
                attrs.clone(),
                Box::new(resolve(input, aliases, visiting)?),
                Box::new(resolve(output, aliases, visiting)?),
            )),
            _ => Ok(tpe.clone()),
        }
    }
    resolve(tpe, aliases, &mut BTreeSet::new())
}

pub(crate) fn validate_function(
    definition: &ValueDefinition,
    aliases: &TupleAliases,
    signatures: &Signatures,
) -> Outcome<()> {
    annotate_function(definition, aliases, signatures).map(|_| ())
}

#[derive(Clone, Copy)]
pub(crate) enum Comparison {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

impl Comparison {
    pub(crate) fn python(self) -> &'static str {
        match self {
            Self::Equal => "==",
            Self::NotEqual => "!=",
            Self::Less => "<",
            Self::LessEqual => "<=",
            Self::Greater => ">",
            Self::GreaterEqual => ">=",
        }
    }

    pub(crate) fn reference(self) -> FQName {
        let name = match self {
            Self::Equal => "equal",
            Self::NotEqual => "not-equal",
            Self::Less => "less-than",
            Self::LessEqual => "less-than-or-equal",
            Self::Greater => "greater-than",
            Self::GreaterEqual => "greater-than-or-equal",
        };
        FQName::from_canonical_string(&format!("morphir/SDK:basics#{name}")).expect("SDK name")
    }

    fn from_reference(reference: &FQName) -> Outcome<Self> {
        [
            Self::Equal,
            Self::NotEqual,
            Self::Less,
            Self::LessEqual,
            Self::Greater,
            Self::GreaterEqual,
        ]
        .into_iter()
        .find(|op| op.reference() == *reference)
        .ok_or_else(|| unsupported("Only scalar comparison applications are supported"))
    }

    pub(crate) fn apply(self, left: Value, right: Value) -> Value {
        Value::Apply(
            Default::default(),
            Box::new(Value::Apply(
                Default::default(),
                Box::new(Value::Reference(Default::default(), self.reference())),
                Box::new(left),
            )),
            Box::new(right),
        )
    }
}

pub(crate) fn scalar(name: &str) -> Type {
    let path = if name == "string" { "string" } else { "basics" };
    Type::Reference(
        Default::default(),
        FQName::from_canonical_string(&format!("morphir/SDK:{path}#{name}")).expect("SDK type"),
        vec![],
    )
}

pub(crate) fn unsupported(message: &str) -> morphir_extension_sdk::Diagnostic {
    error("PY004", message)
}

pub(crate) fn require_type(actual: &Type, expected: &Type) -> Outcome<()> {
    if actual != expected {
        return Err(unsupported(
            "Function expression does not match the required type",
        ));
    }
    Ok(())
}

pub(crate) fn literal_type(literal: &Literal) -> Outcome<Type> {
    Ok(scalar(match literal {
        Literal::Bool(_) => "bool",
        Literal::Integer(_) => "int",
        Literal::Float(_) => "float",
        Literal::String(_) => "string",
        _ => {
            return Err(unsupported(
                "Only bool, int, float and str literals are supported",
            ));
        }
    }))
}

pub(crate) fn require_empty_attributes(value: &Value) -> Outcome<()> {
    if *value.attributes() != ValueAttributes::default() {
        return Err(unsupported(
            "Value attributes cannot yet be preserved in Python",
        ));
    }
    Ok(())
}

pub(crate) fn comparison(value: &Value) -> Outcome<(Comparison, &Value, &Value)> {
    let Value::Apply(_, function, right) = value else {
        return Err(unsupported("Expected a comparison"));
    };
    require_empty_attributes(function)?;
    let Value::Apply(_, reference, left) = function.as_ref() else {
        return Err(unsupported("Expected two comparison arguments"));
    };
    require_empty_attributes(reference)?;
    let Value::Reference(_, reference) = reference.as_ref() else {
        return Err(unsupported("Expected an SDK comparison reference"));
    };
    Ok((Comparison::from_reference(reference)?, left, right))
}

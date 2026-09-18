//! Constructor-aware pattern validation and exhaustiveness for the supported Rust subset.
mod coverage;
pub(crate) use coverage::exhaustive;

use morphir_core::{
    ir::v4::{Literal, Pattern, Type},
    naming::{FQName, Name},
};
use std::collections::BTreeMap;

#[derive(Clone, Default)]
pub(crate) struct Context {
    pub types: BTreeMap<String, CustomType>,
}

#[derive(Clone)]
pub(crate) struct CustomType {
    pub parameters: Vec<Name>,
    pub constructors: Vec<Constructor>,
}

#[derive(Clone)]
pub(crate) struct Constructor {
    pub name: FQName,
    pub arguments: Vec<Type>,
}

impl Context {
    pub fn constructors(&self, ty: &Type) -> Result<Option<Vec<Constructor>>, String> {
        let Type::Reference(_, name, arguments) = ty else {
            return Ok(None);
        };
        let key = name.to_canonical_string();
        let sdk = |name: &str, arguments: Vec<Type>| Constructor {
            name: FQName::from_canonical_string(name).expect("known SDK constructor"),
            arguments,
        };
        match key.as_str() {
            "morphir/SDK:maybe#maybe" if arguments.len() == 1 => {
                return Ok(Some(vec![
                    sdk("morphir/SDK:maybe#nothing", vec![]),
                    sdk("morphir/SDK:maybe#just", vec![arguments[0].clone()]),
                ]));
            }
            "morphir/SDK:result#result" if arguments.len() == 2 => {
                return Ok(Some(vec![
                    sdk("morphir/SDK:result#ok", vec![arguments[1].clone()]),
                    sdk("morphir/SDK:result#err", vec![arguments[0].clone()]),
                ]));
            }
            "morphir/SDK:maybe#maybe" | "morphir/SDK:result#result" => {
                return Err("Invalid SDK type arity in match subject".into());
            }
            _ => {}
        }
        let Some(definition) = self.types.get(&key) else {
            return Ok(None);
        };
        if definition.parameters.len() != arguments.len() {
            return Err(format!("Incorrect type arguments for {key}"));
        }
        let substitutions = definition
            .parameters
            .iter()
            .zip(arguments)
            .map(|(name, ty)| (name.to_canonical_string(), ty.clone()))
            .collect();
        definition
            .constructors
            .iter()
            .map(|constructor| {
                Ok(Constructor {
                    name: constructor.name.clone(),
                    arguments: constructor
                        .arguments
                        .iter()
                        .map(|ty| substitute(ty, &substitutions))
                        .collect::<Result<_, _>>()?,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }
}

fn substitute(ty: &Type, variables: &BTreeMap<String, Type>) -> Result<Type, String> {
    Ok(match ty {
        Type::Variable(_, name) => variables
            .get(&name.to_canonical_string())
            .cloned()
            .ok_or_else(|| format!("Unbound constructor type parameter {name}"))?,
        Type::Reference(attrs, name, args) => Type::Reference(
            attrs.clone(),
            name.clone(),
            args.iter()
                .map(|ty| substitute(ty, variables))
                .collect::<Result<_, _>>()?,
        ),
        Type::Tuple(attrs, fields) => Type::Tuple(
            attrs.clone(),
            fields
                .iter()
                .map(|ty| substitute(ty, variables))
                .collect::<Result<_, _>>()?,
        ),
        Type::Unit(_) => ty.clone(),
        Type::Function(attrs, input, output) => Type::Function(
            attrs.clone(),
            Box::new(substitute(input, variables)?),
            Box::new(substitute(output, variables)?),
        ),
        Type::Record(attrs, fields) => {
            Type::Record(attrs.clone(), substitute_fields(fields, variables)?)
        }
        Type::ExtensibleRecord(..) => {
            return Err("Extensible constructor payloads are not supported in matches".into());
        }
    })
}

fn substitute_fields(
    fields: &[morphir_core::ir::v4::Field],
    variables: &BTreeMap<String, Type>,
) -> Result<Vec<morphir_core::ir::v4::Field>, String> {
    fields
        .iter()
        .map(|field| {
            Ok(morphir_core::ir::v4::Field {
                name: field.name.clone(),
                tpe: substitute(&field.tpe, variables)?,
            })
        })
        .collect()
}

pub(crate) fn bindings(
    pattern: &Pattern,
    subject: &Type,
    context: &Context,
) -> Result<BTreeMap<String, Type>, String> {
    let mut result = BTreeMap::new();
    collect_bindings(pattern, subject, context, &mut result)?;
    Ok(result)
}

fn collect_bindings(
    pattern: &Pattern,
    subject: &Type,
    context: &Context,
    result: &mut BTreeMap<String, Type>,
) -> Result<(), String> {
    if let Some(annotation) = &pattern.attributes().inferred_type
        && !crate::values::same_type(annotation, subject)
    {
        return Err("Pattern annotation does not match subject type".into());
    }
    match pattern {
        Pattern::WildcardPattern(_) => Ok(()),
        Pattern::AsPattern(_, inner, name) if matches!(**inner, Pattern::WildcardPattern(_)) => {
            collect_bindings(inner, subject, context, result)?;
            if result
                .insert(name.to_canonical_string(), subject.clone())
                .is_some()
            {
                return Err(format!("Duplicate pattern binding {name}"));
            }
            Ok(())
        }
        Pattern::TuplePattern(_, fields) => {
            let types = match subject {
                Type::Tuple(_, types) => types.as_slice(),
                Type::Unit(_) if fields.is_empty() => &[],
                _ => return Err("Tuple pattern requires a tuple subject".into()),
            };
            if fields.len() != types.len() {
                return Err("Tuple pattern arity mismatch".into());
            }
            for (field, ty) in fields.iter().zip(types) {
                collect_bindings(field, ty, context, result)?;
            }
            Ok(())
        }
        Pattern::UnitPattern(_)
            if crate::values::same_type(subject, &Type::Unit(Default::default())) =>
        {
            Ok(())
        }
        Pattern::LiteralPattern(_, literal) => {
            let ty = match literal {
                Literal::Bool(_) => crate::values::scalar("basics", "bool"),
                Literal::Char(_) => crate::values::scalar("char", "char"),
                Literal::Integer(value) if value.to_string().parse::<i64>().is_ok() => {
                    crate::values::scalar("basics", "int")
                }
                _ => {
                    return Err(
                        "Only Boolean, i64 and character literal patterns are supported".into(),
                    );
                }
            };
            if !crate::values::same_type(subject, &ty) {
                return Err("Literal pattern does not match subject type".into());
            }
            Ok(())
        }
        Pattern::ConstructorPattern(_, name, fields) => {
            let family = context
                .constructors(subject)?
                .ok_or("Constructor pattern requires a known custom or SDK type")?;
            let constructor = family
                .iter()
                .find(|constructor| constructor.name == *name)
                .ok_or_else(|| format!("Constructor {name} does not belong to the matched type"))?;
            if fields.len() != constructor.arguments.len() {
                return Err(format!("Incorrect pattern arity for {name}"));
            }
            for (field, ty) in fields.iter().zip(&constructor.arguments) {
                collect_bindings(field, ty, context, result)?;
            }
            Ok(())
        }
        _ => Err("Unsupported Rust pattern".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_core::ir::v4::Literal;

    fn boolean(value: bool) -> Pattern {
        Pattern::LiteralPattern(Default::default(), Literal::Bool(value))
    }
    fn wildcard() -> Pattern {
        Pattern::WildcardPattern(Default::default())
    }
    fn variable(name: &str) -> Pattern {
        Pattern::AsPattern(
            Default::default(),
            Box::new(wildcard()),
            Name::from_canonical_string(name).unwrap(),
        )
    }

    #[test]
    fn rejects_nested_coverage_gaps_but_accepts_complete_product() {
        let ty = Type::Tuple(
            Default::default(),
            vec![crate::values::scalar("basics", "bool"); 2],
        );
        let row = |a, b| Pattern::TuplePattern(Default::default(), vec![a, b]);
        let mut cases = vec![
            row(boolean(true), wildcard()),
            row(wildcard(), boolean(true)),
        ];
        assert!(exhaustive(&ty, &cases, &Context::default()).is_err());
        cases.push(row(boolean(false), boolean(false)));
        assert!(exhaustive(&ty, &cases, &Context::default()).is_ok());
    }

    #[test]
    fn binds_instantiated_sdk_payloads_and_rejects_duplicate_bindings() {
        let integer = crate::values::scalar("basics", "int");
        let ty = Type::Reference(
            Default::default(),
            FQName::from_canonical_string("morphir/SDK:maybe#maybe").unwrap(),
            vec![integer.clone()],
        );
        let pattern = Pattern::ConstructorPattern(
            Default::default(),
            FQName::from_canonical_string("morphir/SDK:maybe#just").unwrap(),
            vec![variable("value")],
        );
        assert_eq!(
            bindings(&pattern, &ty, &Context::default())
                .unwrap()
                .get("value"),
            Some(&integer)
        );
        let tuple = Type::Tuple(Default::default(), vec![integer; 2]);
        let duplicate =
            Pattern::TuplePattern(Default::default(), vec![variable("x"), variable("x")]);
        assert!(bindings(&duplicate, &tuple, &Context::default()).is_err());
    }

    #[test]
    fn recursive_wildcard_columns_do_not_expand_indefinitely() {
        let name = FQName::from_canonical_string("test:models#tree").unwrap();
        let tree = Type::Reference(Default::default(), name.clone(), vec![]);
        let context = Context {
            types: BTreeMap::from([(
                name.to_canonical_string(),
                CustomType {
                    parameters: vec![],
                    constructors: vec![
                        Constructor {
                            name: FQName::from_canonical_string("test:models#branch").unwrap(),
                            arguments: vec![tree.clone()],
                        },
                        Constructor {
                            name: FQName::from_canonical_string("test:models#leaf").unwrap(),
                            arguments: vec![],
                        },
                    ],
                },
            )]),
        };
        let ty = Type::Tuple(
            Default::default(),
            vec![tree, crate::values::scalar("basics", "bool")],
        );
        let cases = [false, true].map(|value| {
            Pattern::TuplePattern(Default::default(), vec![wildcard(), boolean(value)])
        });
        assert!(exhaustive(&ty, &cases, &context).is_ok());
        assert!(exhaustive(&ty, &cases[..1], &context).is_err());
    }
}

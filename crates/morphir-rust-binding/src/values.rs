//! Semantic checks shared by Rust function compilation and generation.
use morphir_core::{
    ir::v4::{Literal, Type, Value, ValueBody, ValueDefinition},
    naming::FQName,
};
use std::collections::BTreeMap;

type Environment = BTreeMap<String, Type>;

pub(crate) fn validate_function(
    definition: &ValueDefinition,
    context: &crate::patterns::Context,
) -> Result<(), String> {
    let ValueBody::Expression(body) = &definition.body else {
        return Err("Only expression bodies can be validated as Rust functions".into());
    };
    let output = definition
        .output_type
        .as_ref()
        .ok_or("Function output type is required")?;
    let environment = definition
        .input_types
        .iter()
        .map(|(n, t)| (n.clone(), t.clone()))
        .collect();
    require_type(&infer(body, &environment, context)?, output)
}

pub(crate) fn same_type(left: &Type, right: &Type) -> bool {
    match (left, right) {
        (Type::Unit(_), Type::Unit(_)) => true,
        (Type::Unit(_), Type::Tuple(_, fields)) | (Type::Tuple(_, fields), Type::Unit(_)) => {
            fields.is_empty()
        }
        (Type::Variable(_, a), Type::Variable(_, b)) => a == b,
        (Type::Reference(_, a, aa), Type::Reference(_, b, ba)) => a == b && same_types(aa, ba),
        (Type::Tuple(_, a), Type::Tuple(_, b)) => same_types(a, b),
        (Type::Function(_, a, b), Type::Function(_, c, d)) => same_type(a, c) && same_type(b, d),
        (Type::Record(_, a), Type::Record(_, b)) => same_fields(a, b),
        (Type::ExtensibleRecord(_, a, af), Type::ExtensibleRecord(_, b, bf)) => {
            a == b && same_fields(af, bf)
        }
        _ => false,
    }
}

fn same_types(left: &[Type], right: &[Type]) -> bool {
    left.len() == right.len() && left.iter().zip(right).all(|(a, b)| same_type(a, b))
}

fn same_fields(
    left: &[morphir_core::ir::v4::Field],
    right: &[morphir_core::ir::v4::Field],
) -> bool {
    left.len() == right.len()
        && left.iter().all(|a| {
            right
                .iter()
                .any(|b| a.name == b.name && same_type(&a.tpe, &b.tpe))
        })
}

fn require_type(actual: &Type, expected: &Type) -> Result<(), String> {
    if same_type(actual, expected) {
        Ok(())
    } else {
        Err(format!(
            "Value type {actual:?} does not match expected type {expected:?}"
        ))
    }
}

pub(crate) fn scalar(module: &str, name: &str) -> Type {
    Type::Reference(
        Default::default(),
        FQName::from_canonical_string(&format!("morphir/SDK:{module}#{name}"))
            .expect("known SDK type"),
        vec![],
    )
}

pub(crate) fn infer(
    value: &Value,
    environment: &Environment,
    context: &crate::patterns::Context,
) -> Result<Type, String> {
    let result = match value {
        Value::Unit(_) => Type::Unit(Default::default()),
        Value::Literal(_, literal) => match literal {
            Literal::Bool(_) => scalar("basics", "bool"),
            Literal::Char(_) => scalar("char", "char"),
            Literal::String(_) => scalar("string", "string"),
            Literal::Integer(number) => {
                number
                    .to_string()
                    .parse::<i64>()
                    .map_err(|_| "Integer value is outside the supported i64 range")?;
                scalar("basics", "int")
            }
            Literal::Float(number) if number.value().is_finite() => scalar("basics", "float"),
            _ => return Err("Unsupported Rust literal".into()),
        },
        Value::Variable(_, name) => environment
            .get(&name.to_canonical_string())
            .cloned()
            .ok_or_else(|| format!("Unknown value {name}"))?,
        Value::Tuple(_, elements) => Type::Tuple(
            Default::default(),
            elements
                .iter()
                .map(|element| infer(element, environment, context))
                .collect::<Result<_, _>>()?,
        ),
        Value::IfThenElse(_, condition, yes, no) => {
            require_type(
                &infer(condition, environment, context)?,
                &scalar("basics", "bool"),
            )?;
            let result = infer(yes, environment, context)?;
            require_type(&infer(no, environment, context)?, &result)?;
            result
        }
        Value::PatternMatch(_, subject, cases) => {
            let subject_type = infer(subject, environment, context)?;
            if cases.is_empty() {
                return Err("Empty matches are not supported".into());
            }
            let patterns = cases.iter().map(|case| case.0.clone()).collect::<Vec<_>>();
            crate::patterns::exhaustive(&subject_type, &patterns, context)?;
            let mut result = None;
            for case in cases {
                let mut scope = environment.clone();
                scope.extend(crate::patterns::bindings(&case.0, &subject_type, context)?);
                let arm_type = infer(&case.1, &scope, context)?;
                if let Some(expected) = &result {
                    require_type(&arm_type, expected)?;
                } else {
                    result = Some(arm_type);
                }
            }
            result.expect("nonempty cases")
        }
        Value::LetDefinition(_, name, definition, continuation) => {
            if !definition.input_types.is_empty() {
                return Err("Local function definitions are not supported".into());
            }
            let ValueBody::Expression(body) = &definition.body else {
                return Err("Local bindings require expression bodies".into());
            };
            let inferred = infer(body, environment, context)?;
            let declared = definition
                .output_type
                .as_ref()
                .ok_or("Local binding output type is required")?;
            require_type(&inferred, declared)?;
            let mut scope = environment.clone();
            scope.insert(name.to_canonical_string(), declared.clone());
            infer(continuation, &scope, context)?
        }
        Value::Apply(..) => {
            let (operator, left, right) = comparison(value)?;
            let left_type = infer(left, environment, context)?;
            require_type(&infer(right, environment, context)?, &left_type)?;
            if !operator.accepts(&left_type) {
                return Err(
                    "Comparisons require supported scalar operands of the same type".into(),
                );
            }
            scalar("basics", "bool")
        }
        _ => return Err("Unsupported Rust value expression".into()),
    };
    if let Some(annotation) = &value.attributes().inferred_type {
        require_type(&result, annotation)?;
    }
    Ok(result)
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Comparison {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

impl Comparison {
    pub(crate) fn symbol(self) -> &'static str {
        match self {
            Self::Equal => "==",
            Self::NotEqual => "!=",
            Self::Less => "<",
            Self::LessEqual => "<=",
            Self::Greater => ">",
            Self::GreaterEqual => ">=",
        }
    }

    fn accepts(self, ty: &Type) -> bool {
        [
            ("basics", "int"),
            ("basics", "float"),
            ("char", "char"),
            ("string", "string"),
        ]
        .iter()
        .any(|(module, name)| same_type(ty, &scalar(module, name)))
            || (matches!(self, Self::Equal | Self::NotEqual)
                && same_type(ty, &scalar("basics", "bool")))
    }
}

pub(crate) fn comparison(value: &Value) -> Result<(Comparison, &Value, &Value), String> {
    let Value::Apply(_, function, right) = value else {
        return Err("Expected scalar comparison".into());
    };
    let Value::Apply(_, reference, left) = &**function else {
        return Err("Expected binary scalar comparison".into());
    };
    let Value::Reference(_, reference) = &**reference else {
        return Err("Only SDK scalar comparison calls are supported".into());
    };
    let operator = match reference.to_canonical_string().as_str() {
        "morphir/SDK:basics#equal" => Comparison::Equal,
        "morphir/SDK:basics#not-equal" => Comparison::NotEqual,
        "morphir/SDK:basics#less-than" => Comparison::Less,
        "morphir/SDK:basics#less-than-or-equal" => Comparison::LessEqual,
        "morphir/SDK:basics#greater-than" => Comparison::Greater,
        "morphir/SDK:basics#greater-than-or-equal" => Comparison::GreaterEqual,
        _ => return Err("Only SDK scalar comparison calls are supported".into()),
    };
    Ok((operator, left, right))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn validate_function(definition: &ValueDefinition) -> Result<(), String> {
        super::validate_function(definition, &crate::patterns::Context::default())
    }
    use morphir_core::ir::v4::{Literal, Type, Value, ValueBody};

    fn integer(value: i64) -> Value {
        Value::Literal(Default::default(), Literal::Integer(value.into()))
    }

    #[test]
    fn validates_exhaustive_matches_and_rejects_missing_arms() {
        use morphir_core::ir::v4::{Pattern, PatternCase};
        let subject = Value::Literal(Default::default(), Literal::Bool(true));
        let case = |value, body| {
            PatternCase(
                Pattern::LiteralPattern(Default::default(), Literal::Bool(value)),
                body,
            )
        };
        let body = Value::PatternMatch(
            Default::default(),
            Box::new(subject.clone()),
            vec![case(true, integer(1)), case(false, integer(2))],
        );
        assert!(validate_function(&definition(body)).is_ok());
        let incomplete = Value::PatternMatch(
            Default::default(),
            Box::new(subject),
            vec![case(true, integer(1))],
        );
        assert!(validate_function(&definition(incomplete)).is_err());
    }

    #[test]
    fn match_bindings_are_arm_local_and_annotations_are_checked() {
        use morphir_core::{
            ir::v4::{Pattern, PatternCase},
            naming::Name,
        };
        let name = Name::from_canonical_string("bound").unwrap();
        let variable = Value::Variable(Default::default(), name.clone());
        let bound = Pattern::AsPattern(
            Default::default(),
            Box::new(Pattern::WildcardPattern(Default::default())),
            name,
        );
        let matched = Value::PatternMatch(
            Default::default(),
            Box::new(integer(42)),
            vec![PatternCase(bound.clone(), variable.clone())],
        );
        assert!(validate_function(&definition(matched.clone())).is_ok());
        let leaked = Value::Tuple(Default::default(), vec![matched, variable.clone()]);
        assert!(
            infer(
                &leaked,
                &BTreeMap::new(),
                &crate::patterns::Context::default()
            )
            .unwrap_err()
            .contains("Unknown value")
        );
        let sibling = Value::PatternMatch(
            Default::default(),
            Box::new(integer(42)),
            vec![
                PatternCase(bound, integer(1)),
                PatternCase(Pattern::WildcardPattern(Default::default()), variable),
            ],
        );
        assert!(
            validate_function(&definition(sibling))
                .unwrap_err()
                .contains("Unknown value")
        );
        let attributes = morphir_core::ir::v4::ValueAttributes {
            inferred_type: Some(Box::new(scalar("basics", "bool"))),
            ..Default::default()
        };
        let incorrect = Value::PatternMatch(
            Default::default(),
            Box::new(integer(42)),
            vec![PatternCase(
                Pattern::WildcardPattern(attributes),
                integer(1),
            )],
        );
        assert!(
            validate_function(&definition(incorrect))
                .unwrap_err()
                .contains("annotation")
        );
    }

    fn definition(body: Value) -> ValueDefinition {
        ValueDefinition {
            input_types: Default::default(),
            output_type: Some(Type::Reference(
                Default::default(),
                morphir_core::naming::FQName::from_canonical_string("morphir/SDK:basics#int")
                    .unwrap(),
                vec![],
            )),
            body: ValueBody::Expression(body),
        }
    }

    #[test]
    fn rejects_non_boolean_conditions_and_wrong_result_types() {
        let conditional = Value::IfThenElse(
            Default::default(),
            Box::new(integer(1)),
            Box::new(integer(2)),
            Box::new(integer(3)),
        );
        assert!(validate_function(&definition(conditional)).is_err());
        assert!(validate_function(&definition(Value::Unit(Default::default()))).is_err());
    }

    #[test]
    fn rejects_undefined_variables_and_out_of_range_integers() {
        assert!(
            validate_function(&definition(Value::Variable(
                Default::default(),
                morphir_core::naming::Name::from_canonical_string("missing").unwrap()
            )))
            .is_err()
        );
        let too_large = Value::Literal(
            Default::default(),
            Literal::Integer("9223372036854775808".parse().unwrap()),
        );
        assert!(validate_function(&definition(too_large)).is_err());
    }

    #[test]
    fn validates_lexical_let_scope_and_rejects_leaked_names() {
        let name = morphir_core::naming::Name::from_canonical_string("local").unwrap();
        let variable = Value::Variable(Default::default(), name.clone());
        let local = Value::LetDefinition(
            Default::default(),
            name,
            Box::new(definition(integer(i64::MIN))),
            Box::new(variable.clone()),
        );
        assert!(validate_function(&definition(local.clone())).is_ok());
        let tuple = Value::Tuple(Default::default(), vec![local, variable]);
        let mut f = definition(tuple);
        f.output_type = Some(Type::Tuple(
            Default::default(),
            vec![scalar("basics", "int"); 2],
        ));
        assert!(validate_function(&f).unwrap_err().contains("Unknown value"));
    }

    #[test]
    fn comparisons_validate_both_operands_and_declared_annotations() {
        let reference = Value::Reference(
            Default::default(),
            FQName::from_canonical_string("morphir/SDK:basics#less-than").unwrap(),
        );
        let comparison = Value::Apply(
            Default::default(),
            Box::new(Value::Apply(
                Default::default(),
                Box::new(reference),
                Box::new(integer(1)),
            )),
            Box::new(integer(2)),
        );
        let mut f = definition(comparison.clone());
        f.output_type = Some(scalar("basics", "bool"));
        assert!(validate_function(&f).is_ok());
        let mut environment = BTreeMap::new();
        let mut annotated = scalar("basics", "int");
        if let Type::Reference(attrs, _, _) = &mut annotated {
            attrs.extensions.insert("test".into(), true.into());
        }
        environment.insert("x".into(), annotated);
        let variable = Value::Variable(
            Default::default(),
            morphir_core::naming::Name::from_canonical_string("x").unwrap(),
        );
        assert!(same_type(
            &infer(
                &variable,
                &environment,
                &crate::patterns::Context::default()
            )
            .unwrap(),
            &scalar("basics", "int")
        ));
        let wrong = Value::Literal(
            morphir_core::ir::v4::ValueAttributes {
                inferred_type: Some(Box::new(scalar("basics", "bool"))),
                ..Default::default()
            },
            Literal::Integer(1.into()),
        );
        assert!(validate_function(&definition(wrong)).is_err());
    }
}

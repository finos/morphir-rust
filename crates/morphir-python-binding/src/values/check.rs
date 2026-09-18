//! Bidirectional checking supplies lambda input types from the enclosing annotation.
use super::*;

pub(crate) fn annotate_function(
    definition: &ValueDefinition,
    aliases: &TupleAliases,
    signatures: &Signatures,
) -> Outcome<Value> {
    let ValueBody::Expression(body) = &definition.body else {
        return Err(unsupported("Only expression function bodies are supported"));
    };
    let output = resolve_aliases(&specification(definition)?.output, aliases)?;
    let parameters = definition
        .input_types
        .iter()
        .map(|(name, tpe)| Ok((name.clone(), resolve_aliases(tpe, aliases)?)))
        .collect::<Outcome<_>>()?;
    let signatures = signatures
        .iter()
        .map(|(name, signature)| {
            Ok((
                name.clone(),
                resolve_aliases(&signature_type(signature), aliases)?,
            ))
        })
        .collect::<Outcome<_>>()?;
    check(body, Some(&output), &parameters, &signatures)
}

fn attrs(tpe: Type) -> ValueAttributes {
    ValueAttributes {
        inferred_type: Some(Box::new(tpe)),
        ..Default::default()
    }
}
fn tpe(value: &Value) -> Type {
    *value
        .attributes()
        .inferred_type
        .clone()
        .expect("checked expression")
}

pub(crate) fn binder(pattern: &Pattern) -> Outcome<&Name> {
    let Pattern::AsPattern(a, inner, name) = pattern else {
        return Err(unsupported("Lambda patterns must bind one named variable"));
    };
    if *a != ValueAttributes::default()
        || !matches!(inner.as_ref(), Pattern::WildcardPattern(a) if *a == ValueAttributes::default())
    {
        return Err(unsupported(
            "Lambda patterns must be an unannotated wildcard with a name",
        ));
    }
    crate::names::field_name(name)?;
    Ok(name)
}

fn check(
    value: &Value,
    expected: Option<&Type>,
    parameters: &BTreeMap<String, Type>,
    signatures: &BTreeMap<String, Type>,
) -> Outcome<Value> {
    require_empty_attributes(value)?;
    let result = match value {
        Value::Literal(_, literal) => {
            Value::Literal(attrs(literal_type(literal)?), literal.clone())
        }
        Value::Variable(_, name) => Value::Variable(
            attrs(
                parameters
                    .get(&name.to_canonical_string())
                    .cloned()
                    .ok_or_else(|| unsupported("Unknown function parameter"))?,
            ),
            name.clone(),
        ),
        Value::Reference(_, name) => Value::Reference(
            attrs(
                signatures
                    .get(&name.to_canonical_string())
                    .cloned()
                    .ok_or_else(|| unsupported("Unknown function reference"))?,
            ),
            name.clone(),
        ),
        Value::Lambda(_, pattern, body) => {
            let Some(Type::Function(_, input, output)) = expected else {
                return Err(unsupported(
                    "A lambda needs a contextual unary Callable type",
                ));
            };
            let name = binder(pattern)?;
            let mut scope = parameters.clone();
            scope.insert(name.to_canonical_string(), *input.clone());
            let body = check(body, Some(output), &scope, signatures)?;
            let pattern = Pattern::AsPattern(
                attrs(*input.clone()),
                Box::new(Pattern::WildcardPattern(attrs(*input.clone()))),
                name.clone(),
            );
            Value::Lambda(attrs(expected.unwrap().clone()), pattern, Box::new(body))
        }
        Value::Tuple(_, elements) if elements.len() >= 2 => {
            let types = match expected {
                Some(Type::Tuple(_, types)) if types.len() == elements.len() => Some(types),
                _ => None,
            };
            let elements = elements
                .iter()
                .enumerate()
                .map(|(i, element)| check(element, types.map(|ts| &ts[i]), parameters, signatures))
                .collect::<Outcome<Vec<_>>>()?;
            Value::Tuple(
                attrs(Type::Tuple(
                    Default::default(),
                    elements.iter().map(tpe).collect(),
                )),
                elements,
            )
        }
        Value::IfThenElse(_, condition, yes, no) => {
            let condition = check(condition, Some(&scalar("bool")), parameters, signatures)?;
            let yes = check(yes, expected, parameters, signatures)?;
            let no = check(no, Some(&tpe(&yes)), parameters, signatures)?;
            Value::IfThenElse(
                attrs(tpe(&yes)),
                Box::new(condition),
                Box::new(yes),
                Box::new(no),
            )
        }
        Value::Apply(_, function, argument) => {
            if let Ok((op, left, right)) = comparison(value) {
                let left = check(left, None, parameters, signatures)?;
                let operand = tpe(&left);
                let right = check(right, Some(&operand), parameters, signatures)?;
                let allowed = ["int", "float", "string"]
                    .iter()
                    .any(|name| operand == scalar(name))
                    || (matches!(op, Comparison::Equal | Comparison::NotEqual)
                        && operand == scalar("bool"));
                if !allowed {
                    return Err(unsupported(
                        "Comparison operands must be matching scalar types; bool supports only equality",
                    ));
                }
                let tail = function_type(operand.clone(), scalar("bool"));
                let reference =
                    Value::Reference(attrs(function_type(operand, tail.clone())), op.reference());
                Value::Apply(
                    attrs(scalar("bool")),
                    Box::new(Value::Apply(
                        attrs(tail),
                        Box::new(reference),
                        Box::new(left),
                    )),
                    Box::new(right),
                )
            } else {
                if let Value::Lambda(_, pattern, body) = function.as_ref() {
                    require_empty_attributes(function)?;
                    let argument = check(argument, None, parameters, signatures)?;
                    let input = tpe(&argument);
                    let name = binder(pattern)?;
                    let mut scope = parameters.clone();
                    scope.insert(name.to_canonical_string(), input.clone());
                    let body = check(body, expected, &scope, signatures)?;
                    let output = tpe(&body);
                    let pattern = Pattern::AsPattern(
                        attrs(input.clone()),
                        Box::new(Pattern::WildcardPattern(attrs(input.clone()))),
                        name.clone(),
                    );
                    let function = Value::Lambda(
                        attrs(function_type(input, output.clone())),
                        pattern,
                        Box::new(body),
                    );
                    return Ok(Value::Apply(
                        attrs(output),
                        Box::new(function),
                        Box::new(argument),
                    ));
                }
                let function = check(function, None, parameters, signatures)?;
                let Type::Function(_, input, output) = tpe(&function) else {
                    return Err(unsupported("Call target must have a function type"));
                };
                let argument = check(argument, Some(&input), parameters, signatures)?;
                Value::Apply(attrs(*output), Box::new(function), Box::new(argument))
            }
        }
        _ => return Err(unsupported("Unsupported Python function expression")),
    };
    if let Some(expected) = expected {
        require_type(&tpe(&result), expected)?;
    }
    Ok(result)
}

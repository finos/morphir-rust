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
    Context {
        signatures,
        aliases,
    }
    .check(body, Some(&output), &parameters)
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

struct Context<'a> {
    signatures: &'a Signatures,
    aliases: &'a TupleAliases,
}

impl Context<'_> {
    fn check(
        &self,
        value: &Value,
        expected: Option<&Type>,
        parameters: &BTreeMap<String, Type>,
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
            Value::Reference(_, name) => {
                // Resolve only referenced signatures: rebuilding the whole package for
                // every body makes WASM validation quadratic in the number of functions.
                let signature = self
                    .signatures
                    .get(&name.to_canonical_string())
                    .ok_or_else(|| unsupported("Unknown function reference"))?;
                Value::Reference(
                    attrs(resolve_aliases(&signature_type(signature), self.aliases)?),
                    name.clone(),
                )
            }
            Value::Lambda(_, pattern, body) => {
                let Some(Type::Function(_, input, output)) = expected else {
                    return Err(unsupported(
                        "A lambda needs a contextual unary Callable type",
                    ));
                };
                let name = binder(pattern)?;
                let mut scope = parameters.clone();
                scope.insert(name.to_canonical_string(), *input.clone());
                let body = self.check(body, Some(output), &scope)?;
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
                    .map(|(i, element)| self.check(element, types.map(|ts| &ts[i]), parameters))
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
                let condition = self.check(condition, Some(&scalar("bool")), parameters)?;
                let yes = self.check(yes, expected, parameters)?;
                let no = self.check(no, Some(&tpe(&yes)), parameters)?;
                Value::IfThenElse(
                    attrs(tpe(&yes)),
                    Box::new(condition),
                    Box::new(yes),
                    Box::new(no),
                )
            }
            Value::Apply(_, function, argument) => {
                if let Ok((op, left, right)) = comparison(value) {
                    let left = self.check(left, None, parameters)?;
                    let operand = tpe(&left);
                    let right = self.check(right, Some(&operand), parameters)?;
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
                    let reference = Value::Reference(
                        attrs(function_type(operand, tail.clone())),
                        op.reference(),
                    );
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
                        let argument = self.check(argument, None, parameters)?;
                        let input = tpe(&argument);
                        let name = binder(pattern)?;
                        let mut scope = parameters.clone();
                        scope.insert(name.to_canonical_string(), input.clone());
                        let body = self.check(body, expected, &scope)?;
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
                    let function = self.check(function, None, parameters)?;
                    let Type::Function(_, input, output) = tpe(&function) else {
                        return Err(unsupported("Call target must have a function type"));
                    };
                    let argument = self.check(argument, Some(&input), parameters)?;
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
}

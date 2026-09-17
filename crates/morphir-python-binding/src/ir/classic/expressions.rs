//! Infer v3 value annotations and verify annotations supplied by a v3 producer.

use super::{Type, fqname, name, tpe};
use crate::{Outcome, values};
use morphir_core::ir::{classic as c, v4 as v};
use std::collections::BTreeMap;

type Value = c::Value<c::Attrs, Type>;

fn annotation(
    value: &v::Value,
    expected: &v::Type,
    aliases: &values::TupleAliases,
) -> Outcome<Type> {
    let attrs = value.attributes();
    if attrs.source.is_some() || !attrs.extensions.is_empty() {
        return Err(values::unsupported(
            "Value metadata cannot be preserved in Python",
        ));
    }
    if let Some(actual) = &attrs.inferred_type
        && values::resolve_aliases(actual, aliases)? != values::resolve_aliases(expected, aliases)?
    {
        return Err(values::unsupported(
            "IR v3 value annotation does not match its expression type",
        ));
    }
    tpe(expected)
}

pub(super) fn encode(
    value: &v::Value,
    parameters: &BTreeMap<String, v::Type>,
    aliases: &values::TupleAliases,
) -> Outcome<Value> {
    let clean = erase(value)?;
    let inferred = values::infer(&clean, parameters)?;
    let attrs = annotation(value, &inferred, aliases)?;
    Ok(match value {
        v::Value::Variable(_, key) => c::Value::Variable(attrs, name(key)?),
        v::Value::Literal(_, literal) => c::Value::Literal(attrs, match literal {
            v::Literal::Bool(value) => c::Literal::Bool(*value),
            v::Literal::Integer(value) => c::Literal::WholeNumber(i64::try_from(value).map_err(|_| values::unsupported("IR v3 whole-number literals must fit the shared codec's signed 64-bit range; use IR v4 for larger integers"))?),
            v::Literal::Float(value) => c::Literal::Float(value.value()),
            v::Literal::String(value) => c::Literal::String(value.clone()),
            _ => return Err(values::unsupported("Unsupported Python literal")),
        }),
        v::Value::Tuple(_, elements) => c::Value::Tuple(attrs, elements.iter().map(|value| encode(value, parameters, aliases)).collect::<Outcome<_>>()?),
        v::Value::IfThenElse(_, condition, yes, no) => c::Value::IfThenElse(attrs,
            Box::new(encode(condition, parameters, aliases)?), Box::new(encode(yes, parameters, aliases)?), Box::new(encode(no, parameters, aliases)?)),
        v::Value::Apply(_, partial, right) => {
            // The shared subset validator has already required a fully applied scalar comparison.
            let v::Value::Apply(_, reference, left) = partial.as_ref() else { unreachable!() };
            let v::Value::Reference(_, key) = reference.as_ref() else { unreachable!() };
            let operand = values::infer(&erase(left)?, parameters)?;
            let tail = v::Type::Function(Default::default(), Box::new(operand.clone()), Box::new(values::scalar("bool")));
            let full = v::Type::Function(Default::default(), Box::new(operand), Box::new(tail.clone()));
            c::Value::Apply(attrs, Box::new(c::Value::Apply(annotation(partial, &tail, aliases)?,
                Box::new(c::Value::Reference(annotation(reference, &full, aliases)?, fqname(key)?)),
                Box::new(encode(left, parameters, aliases)?))), Box::new(encode(right, parameters, aliases)?))
        }
        _ => return Err(values::unsupported("Unsupported Python expression")),
    })
}

/// Remove only inferred types after they have been checked against the shared subset inference.
pub(super) fn erase(value: &v::Value) -> Outcome<v::Value> {
    let a = Default::default();
    Ok(match value {
        v::Value::Variable(_, name) => v::Value::Variable(a, name.clone()),
        v::Value::Reference(_, name) => v::Value::Reference(a, name.clone()),
        v::Value::Literal(_, literal) => v::Value::Literal(a, literal.clone()),
        v::Value::Tuple(_, elements) => {
            v::Value::Tuple(a, elements.iter().map(erase).collect::<Outcome<_>>()?)
        }
        v::Value::Apply(_, function, argument) => {
            v::Value::Apply(a, Box::new(erase(function)?), Box::new(erase(argument)?))
        }
        v::Value::IfThenElse(_, condition, yes, no) => v::Value::IfThenElse(
            a,
            Box::new(erase(condition)?),
            Box::new(erase(yes)?),
            Box::new(erase(no)?),
        ),
        _ => {
            return Err(values::unsupported(
                "IR v3 expression is outside the Python subset",
            ));
        }
    })
}

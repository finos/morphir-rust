//! Infer v3 value annotations and verify annotations supplied by a v3 producer.

use super::{Type, fqname, name, tpe};
use crate::{Outcome, values};
use morphir_core::ir::{classic as c, v4 as v};

type Value = c::Value<c::Attrs, Type>;

fn annotation(
    value: &v::ValueAttributes,
    expected: &v::Type,
    aliases: &values::TupleAliases,
) -> Outcome<Type> {
    let attrs = value;
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
    typed: &v::Value,
    aliases: &values::TupleAliases,
) -> Outcome<Value> {
    let inferred = typed
        .attributes()
        .inferred_type
        .as_ref()
        .expect("checked expression");
    let attrs = annotation(value.attributes(), inferred, aliases)?;
    Ok(match value {
        v::Value::Variable(_, key) => c::Value::Variable(attrs, name(key)?),
        v::Value::Literal(_, literal) => c::Value::Literal(attrs, match literal {
            v::Literal::Bool(value) => c::Literal::Bool(*value),
            v::Literal::Integer(value) => c::Literal::WholeNumber(i64::try_from(value).map_err(|_| values::unsupported("IR v3 whole-number literals must fit the shared codec's signed 64-bit range; use IR v4 for larger integers"))?),
            v::Literal::Float(value) => c::Literal::Float(value.value()),
            v::Literal::String(value) => c::Literal::String(value.clone()),
            _ => return Err(values::unsupported("Unsupported Python literal")),
        }),
        v::Value::Reference(_, key) => c::Value::Reference(attrs, fqname(key)?),
        v::Value::Tuple(_, elements) => {
            let v::Value::Tuple(_, checked) = typed else { unreachable!() };
            c::Value::Tuple(attrs, elements.iter().zip(checked).map(|(value, typed)| encode(value, typed, aliases)).collect::<Outcome<_>>()?)
        }
        v::Value::IfThenElse(_, condition, yes, no) => {
            let v::Value::IfThenElse(_, tc, ty, tn) = typed else { unreachable!() };
            c::Value::IfThenElse(attrs, Box::new(encode(condition, tc, aliases)?), Box::new(encode(yes, ty, aliases)?), Box::new(encode(no, tn, aliases)?))
        }
        v::Value::Apply(_, function, argument) => {
            let v::Value::Apply(_, tf, ta) = typed else { unreachable!() };
            c::Value::Apply(attrs, Box::new(encode(function, tf, aliases)?), Box::new(encode(argument, ta, aliases)?))
        }
        v::Value::Lambda(_, pattern, body) => {
            let v::Value::Lambda(_, tp, tb) = typed else { unreachable!() };
            c::Value::Lambda(attrs, encode_pattern(pattern, tp, aliases)?, Box::new(encode(body, tb, aliases)?))
        }        _ => return Err(values::unsupported("Unsupported Python expression")),
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
        v::Value::Lambda(_, pattern, body) => {
            v::Value::Lambda(a, erase_pattern(pattern)?, Box::new(erase(body)?))
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

fn encode_pattern(
    pattern: &v::Pattern,
    typed: &v::Pattern,
    aliases: &values::TupleAliases,
) -> Outcome<c::Pattern<Type>> {
    let attrs = annotation(
        pattern.attributes(),
        typed
            .attributes()
            .inferred_type
            .as_ref()
            .expect("checked pattern"),
        aliases,
    )?;
    Ok(match (pattern, typed) {
        (v::Pattern::WildcardPattern(_), _) => c::Pattern::Wildcard(attrs),
        (v::Pattern::AsPattern(_, inner, key), v::Pattern::AsPattern(_, checked, _)) => {
            c::Pattern::As(
                attrs,
                Box::new(encode_pattern(inner, checked, aliases)?),
                name(key)?,
            )
        }
        _ => return Err(values::unsupported("Unsupported lambda pattern")),
    })
}
fn erase_pattern(pattern: &v::Pattern) -> Outcome<v::Pattern> {
    Ok(match pattern {
        v::Pattern::WildcardPattern(_) => v::Pattern::WildcardPattern(Default::default()),
        v::Pattern::AsPattern(_, inner, name) => v::Pattern::AsPattern(
            Default::default(),
            Box::new(erase_pattern(inner)?),
            name.clone(),
        ),
        _ => return Err(values::unsupported("Unsupported lambda pattern")),
    })
}

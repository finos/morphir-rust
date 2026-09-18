use super::*;
pub(super) fn substitute(
    ty: &Type<Attrs>,
    variables: &BTreeMap<String, Type<Attrs>>,
) -> Type<Attrs> {
    match ty {
        Type::Variable(_, name) => variables
            .get(&format!("{name:?}"))
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        Type::Function(a, i, o) => Type::Function(
            a.clone(),
            Box::new(substitute(i, variables)),
            Box::new(substitute(o, variables)),
        ),
        Type::Tuple(a, ts) => Type::Tuple(
            a.clone(),
            ts.iter().map(|t| substitute(t, variables)).collect(),
        ),
        Type::Reference(a, n, args) => Type::Reference(
            a.clone(),
            n.clone(),
            args.iter().map(|t| substitute(t, variables)).collect(),
        ),
        _ => ty.clone(),
    }
}
pub(super) fn unify(
    expected: &Type<Attrs>,
    actual: &Type<Attrs>,
    flexible: &BTreeSet<String>,
    variables: &mut BTreeMap<String, Type<Attrs>>,
) -> Result<(), String> {
    if let Type::Variable(_, name) = expected {
        let key = format!("{name:?}");
        if flexible.contains(&key) {
            if let Some(previous) = variables.get(&key) {
                if previous != actual {
                    return Err("Inconsistent generic argument types".into());
                }
            } else {
                variables.insert(key, actual.clone());
            }
            return Ok(());
        }
    }
    match (expected, actual) {
        (Type::Function(_, a, b), Type::Function(_, c, d)) => {
            unify(a, c, flexible, variables)?;
            unify(b, d, flexible, variables)
        }
        (Type::Tuple(_, a), Type::Tuple(_, b)) if a.len() == b.len() => {
            for (a, b) in a.iter().zip(b) {
                unify(a, b, flexible, variables)?;
            }
            Ok(())
        }
        (Type::Reference(_, an, a), Type::Reference(_, bn, b))
            if an == bn && a.len() == b.len() =>
        {
            for (a, b) in a.iter().zip(b) {
                unify(a, b, flexible, variables)?;
            }
            Ok(())
        }
        _ if expected == actual => Ok(()),
        _ => Err("Function argument does not match the declared input type".into()),
    }
}

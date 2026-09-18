//! Pattern-matrix specialization checks nested finite domains without inventing fallback arms.
use super::{Context, bindings};
use morphir_core::{
    ir::v4::{Literal, Pattern, Type},
    naming::FQName,
};

enum Head {
    Boolean(bool),
    Unit,
    Tuple,
    Constructor(FQName),
}
struct Case {
    head: Head,
    arguments: Vec<Type>,
}

pub(crate) fn exhaustive(
    subject: &Type,
    patterns: &[Pattern],
    context: &Context,
) -> Result<(), String> {
    for pattern in patterns {
        bindings(pattern, subject, context)?;
    }
    let rows = patterns
        .iter()
        .map(|pattern| vec![pattern.clone()])
        .collect::<Vec<_>>();
    if covered(std::slice::from_ref(subject), &rows, context)? {
        Ok(())
    } else {
        Err("Match is not exhaustive; add missing cases or a wildcard arm".into())
    }
}

fn wildcard(pattern: &Pattern) -> bool {
    matches!(pattern, Pattern::WildcardPattern(_))
        || matches!(pattern, Pattern::AsPattern(_, inner, _) if wildcard(inner))
}

fn family(ty: &Type, context: &Context) -> Result<Option<Vec<Case>>, String> {
    if crate::values::same_type(ty, &crate::values::scalar("basics", "bool")) {
        return Ok(Some(
            [false, true]
                .into_iter()
                .map(|value| Case {
                    head: Head::Boolean(value),
                    arguments: vec![],
                })
                .collect(),
        ));
    }
    Ok(match ty {
        Type::Unit(_) => Some(vec![Case {
            head: Head::Unit,
            arguments: vec![],
        }]),
        Type::Tuple(_, args) if args.is_empty() => Some(vec![Case {
            head: Head::Unit,
            arguments: vec![],
        }]),
        Type::Tuple(_, args) => Some(vec![Case {
            head: Head::Tuple,
            arguments: args.clone(),
        }]),
        _ => context.constructors(ty)?.map(|family| {
            family
                .into_iter()
                .map(|constructor| Case {
                    head: Head::Constructor(constructor.name),
                    arguments: constructor.arguments,
                })
                .collect()
        }),
    })
}

fn fields(pattern: &Pattern, case: &Case) -> Option<Vec<Pattern>> {
    if wildcard(pattern) {
        return Some(vec![
            Pattern::WildcardPattern(Default::default());
            case.arguments.len()
        ]);
    }
    match (pattern, &case.head) {
        (Pattern::LiteralPattern(_, Literal::Bool(value)), Head::Boolean(expected))
            if value == expected =>
        {
            Some(vec![])
        }
        (Pattern::UnitPattern(_), Head::Unit) => Some(vec![]),
        (Pattern::TuplePattern(_, fields), Head::Unit) if fields.is_empty() => Some(vec![]),
        (Pattern::TuplePattern(_, fields), Head::Tuple) => Some(fields.clone()),
        (Pattern::ConstructorPattern(_, actual, fields), Head::Constructor(expected))
            if actual == expected =>
        {
            Some(fields.clone())
        }
        _ => None,
    }
}

fn covered(types: &[Type], rows: &[Vec<Pattern>], context: &Context) -> Result<bool, String> {
    if rows.iter().any(|row| row.iter().all(wildcard)) {
        return Ok(true);
    }
    let Some((first, rest)) = types.split_first() else {
        return Ok(!rows.is_empty());
    };
    // An unconstrained column needs no constructor expansion, especially for
    // recursive types whose payload would otherwise reproduce this column.
    if rows.iter().all(|row| wildcard(&row[0])) {
        let tails = rows.iter().map(|row| row[1..].to_vec()).collect::<Vec<_>>();
        return covered(rest, &tails, context);
    }
    if let Some(family) = family(first, context)? {
        for case in family {
            let rows = rows
                .iter()
                .filter_map(|row| {
                    fields(&row[0], &case).map(|mut fields| {
                        fields.extend_from_slice(&row[1..]);
                        fields
                    })
                })
                .collect::<Vec<_>>();
            // No arm can cover this constructor, even before considering its payloads.
            if rows.is_empty() {
                return Ok(false);
            }
            let mut types = case.arguments;
            types.extend_from_slice(rest);
            if !covered(&types, &rows, context)? {
                return Ok(false);
            }
        }
        Ok(true)
    } else {
        let defaults = rows
            .iter()
            .filter(|row| wildcard(&row[0]))
            .map(|row| row[1..].to_vec())
            .collect::<Vec<_>>();
        if defaults.is_empty() {
            return Ok(false);
        }
        covered(rest, &defaults, context)
    }
}

use morphir_core::ir::v4::{Pattern, Type, Value, ValueBody};
use std::collections::BTreeSet;

pub(crate) fn copy_type(ty: &Type) -> bool {
    matches!(ty, Type::Unit(_))
        || matches!(ty, Type::Tuple(_, elements) if elements.iter().all(copy_type))
        || [
            ("basics", "int"),
            ("basics", "float"),
            ("basics", "bool"),
            ("char", "char"),
        ]
        .iter()
        .any(|(m, n)| crate::values::same_type(ty, &crate::values::scalar(m, n)))
}
fn bind(pattern: &Pattern, bound: &mut BTreeSet<String>) {
    match pattern {
        Pattern::AsPattern(_, p, n) => {
            bind(p, bound);
            bound.insert(n.to_canonical_string());
        }
        Pattern::TuplePattern(_, ps) | Pattern::ConstructorPattern(_, _, ps) => {
            for p in ps {
                bind(p, bound);
            }
        }
        Pattern::HeadTailPattern(_, a, b) => {
            bind(a, bound);
            bind(b, bound);
        }
        _ => {}
    }
}
pub(crate) fn free_variables(value: &Value) -> BTreeSet<String> {
    fn walk(value: &Value, bound: &BTreeSet<String>, free: &mut BTreeSet<String>) {
        match value {
            Value::Variable(_, n) => {
                let n = n.to_canonical_string();
                if !bound.contains(&n) {
                    free.insert(n);
                }
            }
            Value::Lambda(_, p, body) => {
                let mut scope = bound.clone();
                bind(p, &mut scope);
                walk(body, &scope, free);
            }
            Value::Apply(_, f, a) => {
                walk(f, bound, free);
                walk(a, bound, free);
            }
            Value::Tuple(_, elements) => {
                for e in elements {
                    walk(e, bound, free);
                }
            }
            Value::IfThenElse(_, c, y, n) => {
                walk(c, bound, free);
                walk(y, bound, free);
                walk(n, bound, free);
            }
            Value::LetDefinition(_, n, d, b) => {
                if let ValueBody::Expression(v) = &d.body {
                    let mut scope = bound.clone();
                    scope.extend(d.input_types.keys().cloned());
                    walk(v, &scope, free);
                }
                let mut scope = bound.clone();
                scope.insert(n.to_canonical_string());
                walk(b, &scope, free);
            }
            Value::PatternMatch(_, s, cases) => {
                walk(s, bound, free);
                for case in cases {
                    let mut scope = bound.clone();
                    bind(&case.0, &mut scope);
                    walk(&case.1, &scope, free);
                }
            }
            _ => {}
        }
    }
    let mut result = BTreeSet::new();
    walk(value, &BTreeSet::new(), &mut result);
    result
}

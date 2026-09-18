use morphir_core::ir::v4::{Value, ValueBody, ValueDefinition};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn check_cycles(definitions: &BTreeMap<String, ValueDefinition>) -> Result<(), String> {
    fn references(value: &Value, result: &mut BTreeSet<String>) {
        match value {
            Value::Reference(_, name) => {
                result.insert(name.to_canonical_string());
            }
            Value::Apply(_, f, a) => {
                references(f, result);
                references(a, result);
            }
            Value::Lambda(_, _, body) => references(body, result),
            Value::LetDefinition(_, _, d, b) => {
                if let ValueBody::Expression(v) = &d.body {
                    references(v, result);
                }
                references(b, result);
            }
            Value::IfThenElse(_, c, y, n) => {
                references(c, result);
                references(y, result);
                references(n, result);
            }
            Value::Tuple(_, vs) => {
                for v in vs {
                    references(v, result);
                }
            }
            Value::PatternMatch(_, s, cases) => {
                references(s, result);
                for case in cases {
                    references(&case.1, result);
                }
            }
            _ => {}
        }
    }
    fn visit(
        name: &str,
        edges: &BTreeMap<String, BTreeSet<String>>,
        active: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
    ) -> Result<(), String> {
        if done.contains(name) {
            return Ok(());
        }
        if !active.insert(name.into()) {
            return Err(format!(
                "Recursive function reference cycle involving {name} is unsupported"
            ));
        }
        if let Some(next) = edges.get(name) {
            for name in next {
                visit(name, edges, active, done)?;
            }
        }
        active.remove(name);
        done.insert(name.into());
        Ok(())
    }
    let edges = definitions
        .iter()
        .map(|(name, d)| {
            let mut refs = BTreeSet::new();
            if let ValueBody::Expression(v) = &d.body {
                references(v, &mut refs);
            }
            (name.clone(), refs)
        })
        .collect();
    let mut done = BTreeSet::new();
    for name in definitions.keys() {
        visit(name, &edges, &mut BTreeSet::new(), &mut done)?;
    }
    Ok(())
}

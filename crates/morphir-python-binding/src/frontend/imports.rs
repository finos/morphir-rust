//! Static import resolution. Imports never load or execute Python code.

use super::{TypeScope, declare, union_members};
use crate::{Outcome, error, modules::ModuleIdentity, names};
use morphir_core::{ir::v4::PackageName, naming::FQName};
use ruff_python_ast::{Expr, Stmt};
use std::collections::{BTreeMap, BTreeSet};

pub(super) type Exports = BTreeMap<String, TypeScope>;

pub(super) fn exports(
    statements: &[Stmt],
    package: &PackageName,
    module: &str,
) -> Outcome<TypeScope> {
    let mut types = BTreeSet::new();
    let mut variants = vec![];
    for statement in statements {
        match statement {
            Stmt::ClassDef(class) => {
                types.insert(class.name.to_string());
            }
            Stmt::TypeAlias(alias) => {
                types.insert(super::expr_name(&alias.name)?.to_owned());
                if !matches!(alias.value.as_ref(), Expr::Subscript(_)) {
                    union_members(&alias.value, &mut variants)?;
                }
            }
            _ => {}
        }
    }
    types
        .into_iter()
        .filter(|name| !variants.contains(&name.as_str()))
        .map(|name| {
            let fq = format!(
                "{}:{module}#{}",
                package.to_canonical_string(),
                names::identifier(&name)?.to_canonical_string()
            );
            Ok((
                name,
                FQName::from_canonical_string(&fq).map_err(|e| error("PY003", e))?,
            ))
        })
        .collect()
}

pub(super) fn scope(
    statements: &[Stmt],
    identity: &ModuleIdentity,
    exports: &Exports,
) -> Outcome<TypeScope> {
    let mut types = exports[&identity.python].clone();
    let mut bindings = BTreeSet::new();
    let mut module_bindings = BTreeMap::new();
    for statement in statements {
        match statement {
            Stmt::ClassDef(class) => declare(&mut bindings, class.name.as_str())?,
            Stmt::TypeAlias(alias) => declare(&mut bindings, super::expr_name(&alias.name)?)?,
            Stmt::FunctionDef(function) => declare(&mut bindings, function.name.as_str())?,
            _ => {}
        }
    }
    for (index, statement) in statements.iter().enumerate() {
        match statement {
            Stmt::ImportFrom(import) => {
                let module = import.module.as_ref().map(|m| m.as_str()).unwrap_or("");
                if import.is_lazy {
                    return Err(error("PY004", "Lazy imports are not supported"));
                }
                if import.level == 0 && ["dataclasses", "__future__"].contains(&module) {
                    let expected = if module == "dataclasses" {
                        "dataclass"
                    } else {
                        "annotations"
                    };
                    if import.names.len() != 1
                        || import.names[0].name.as_str() != expected
                        || import.names[0].asname.is_some()
                    {
                        return Err(error(
                            "PY004",
                            "Only dataclasses.dataclass and __future__.annotations standard imports are supported",
                        ));
                    }
                    if module == "__future__" && !statements[..index].iter().all(|s| {
                        matches!(s, Stmt::ImportFrom(i) if i.level == 0 && i.module.as_ref().map(|m| m.as_str()) == Some("__future__"))
                    }) {
                        return Err(error("PY004", "Future imports must precede other statements"));
                    }
                    continue;
                }
                let target = absolute_module(&identity.python, module, import.level)?;
                for alias in &import.names {
                    let name = alias.name.as_str();
                    let bound = alias.asname.as_ref().map(|a| a.as_str()).unwrap_or(name);
                    declare(&mut bindings, bound)?;
                    if let Some(fq) = exports.get(&target).and_then(|types| types.get(name)) {
                        types.insert(bound.into(), fq.clone());
                    } else {
                        let child = if target.is_empty() {
                            name.into()
                        } else {
                            format!("{target}.{name}")
                        };
                        import_module(&mut types, exports, &child, bound)?;
                    }
                }
            }
            Stmt::Import(import) => {
                if import.is_lazy {
                    return Err(error("PY004", "Lazy imports are not supported"));
                }
                for alias in &import.names {
                    let target = alias.name.as_str();
                    let bound = alias.asname.as_ref().map(|a| a.as_str()).unwrap_or(target);
                    let root = bound.split('.').next().unwrap_or(bound);
                    let binding = if alias.asname.is_some() { target } else { root };
                    match module_bindings.get(root) {
                        Some(existing) if existing == &binding => {}
                        _ => {
                            declare(&mut bindings, root)?;
                            module_bindings.insert(root, binding);
                        }
                    }
                    import_module(&mut types, exports, target, bound)?;
                }
            }
            _ => {}
        }
    }
    Ok(types)
}

fn absolute_module(current: &str, module: &str, level: u32) -> Outcome<String> {
    if level == 0 {
        return Ok(module.into());
    }
    let mut segments: Vec<_> = current.split('.').collect();
    if level as usize >= segments.len() {
        return Err(error("PY004", "Relative import escapes the source package"));
    }
    segments.truncate(segments.len() - level as usize);
    if !module.is_empty() {
        segments.push(module);
    }
    Ok(segments.join("."))
}

fn import_module(
    scope: &mut TypeScope,
    exports: &Exports,
    target: &str,
    bound: &str,
) -> Outcome<()> {
    let types = exports.get(target).ok_or_else(|| {
        error(
            "PY004",
            format!("Unresolved local module or type import: {target}"),
        )
    })?;
    for (name, fq) in types {
        scope.insert(format!("{bound}.{name}"), fq.clone());
    }
    Ok(())
}

pub(super) fn qualified_name(expr: &Expr) -> Outcome<String> {
    match expr {
        Expr::Name(name) => Ok(name.id.to_string()),
        Expr::Attribute(attribute) => Ok(format!(
            "{}.{}",
            qualified_name(&attribute.value)?,
            attribute.attr
        )),
        _ => Err(error("PY004", "Expected an imported module and type name")),
    }
}

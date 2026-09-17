//! Generate module-qualified type annotations to avoid imported name collisions.

use crate::{Outcome, error, modules::ModuleIdentity, names};
use morphir_core::ir::v4::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn prepare(
    library: &LibraryContent,
    current: &str,
    identities: &BTreeMap<String, ModuleIdentity>,
    scope: &mut BTreeMap<String, String>,
) -> Outcome<String> {
    let definition = &library.def.modules[current].value;
    let mut used: BTreeSet<_> = definition
        .types
        .keys()
        .chain(definition.values.keys())
        .cloned()
        .collect();
    let mut references = BTreeSet::new();
    for entry in definition.types.values() {
        match &entry.value.value {
            TypeDefinition::TypeAliasDefinition { type_expr, .. } => {
                collect(type_expr, &mut references)
            }
            TypeDefinition::CustomTypeDefinition { constructors, .. } => {
                for constructor in &constructors.value {
                    used.insert(constructor.name.to_canonical_string());
                    for arg in &constructor.args {
                        collect(&arg.arg_type, &mut references);
                    }
                }
            }
            _ => {}
        }
    }
    for entry in definition.values.values() {
        for input in entry.value.value.input_types.values() {
            collect(input, &mut references);
        }
        if let Some(output) = &entry.value.value.output_type {
            collect(output, &mut references);
        }
    }
    let mut source = String::new();
    let mut next_alias = 1;
    for (module, identity) in identities {
        if module == current {
            continue;
        }
        let prefix = format!("{}:{module}#", library.package_name.to_canonical_string());
        if !references.iter().any(|fq| fq.starts_with(&prefix)) {
            continue;
        }
        let alias = loop {
            let candidate = format!("morphir_module_{next_alias}");
            next_alias += 1;
            if used.insert(Name::from(candidate.as_str()).to_canonical_string()) {
                break candidate;
            }
        };
        source.push_str(&format!("import {} as {alias}\n", identity.python));
        for name in library.def.modules[module].value.types.keys() {
            let python = names::type_name(
                &Name::from_canonical_string(name).map_err(|e| error("PY003", e))?,
            )?;
            scope.insert(format!("{prefix}{name}"), format!("{alias}.{python}"));
        }
    }
    if !source.is_empty() {
        source.push('\n');
    }
    Ok(source)
}

fn collect(tpe: &Type, references: &mut BTreeSet<String>) {
    match tpe {
        Type::Reference(_, fq, args) => {
            references.insert(fq.to_canonical_string());
            for arg in args {
                collect(arg, references);
            }
        }
        Type::Tuple(_, items) => {
            for item in items {
                collect(item, references);
            }
        }
        Type::Record(_, fields) => {
            for field in fields {
                collect(&field.tpe, references);
            }
        }
        _ => {}
    }
}

//! Keep recursive Rust structs nominal: recursive Morphir record aliases are invalid.
use super::types::Context;
use crate::Outcome;
use morphir_core::ir::classic::{module::ModuleTypeDefinition, *};
use std::collections::{HashMap, HashSet};

pub(super) fn nominalize(
    context: &Context<'_>,
    items: &[syn::Item],
    definitions: Vec<ModuleTypeDefinition<Attrs>>,
) -> Outcome<Vec<ModuleTypeDefinition<Attrs>>> {
    let graph: HashMap<_, _> = definitions
        .iter()
        .map(|(name, controlled)| {
            let mut references = HashSet::new();
            match &controlled.value.value {
                TypeDefinition::Alias(_, ty) => collect_references(context, ty, &mut references),
                TypeDefinition::Custom(_, constructors) => {
                    for constructor in &constructors.value {
                        for (_, ty) in &constructor.args {
                            collect_references(context, ty, &mut references);
                        }
                    }
                }
            }
            (name.clone(), references)
        })
        .collect();
    let mut recursive_structs = HashSet::new();
    for item in items {
        if let syn::Item::Struct(structure) = item {
            let name = context.source.name(&structure.ident)?;
            if reaches(&graph, &name, &name, &mut HashSet::new()) {
                recursive_structs.insert(name);
            }
        }
    }
    Ok(definitions
        .into_iter()
        .map(|(name, controlled)| {
            let AccessControlled {
                access,
                value: Documented { doc, value },
            } = controlled;
            let definition = match value {
                TypeDefinition::Alias(parameters, Type::Record(_, fields))
                    if recursive_structs.contains(&name) =>
                {
                    TypeDefinition::Custom(
                        parameters,
                        AccessControlled {
                            access: access.clone(),
                            value: vec![Constructor {
                                name: name.clone(),
                                args: fields
                                    .into_iter()
                                    .map(|field| (field.name, field.ty))
                                    .collect(),
                            }],
                        },
                    )
                }
                other => other,
            };
            (
                name,
                AccessControlled {
                    access,
                    value: Documented::new(doc, definition),
                },
            )
        })
        .collect())
}

fn collect_references(context: &Context<'_>, ty: &Type<Attrs>, references: &mut HashSet<Name>) {
    match ty {
        Type::Reference(_, target, args) => {
            if &target.package_path == context.package && &target.module_path == context.module {
                references.insert(target.local_name.clone());
            }
            for arg in args {
                collect_references(context, arg, references);
            }
        }
        Type::Tuple(_, args) => {
            for arg in args {
                collect_references(context, arg, references);
            }
        }
        Type::Record(_, fields) | Type::ExtensibleRecord(_, _, fields) => {
            for field in fields {
                collect_references(context, &field.ty, references);
            }
        }
        Type::Function(_, argument, result) => {
            collect_references(context, argument, references);
            collect_references(context, result, references);
        }
        Type::Unit(_) | Type::Variable(_, _) => {}
    }
}

fn reaches(
    graph: &HashMap<Name, HashSet<Name>>,
    current: &Name,
    target: &Name,
    visited: &mut HashSet<Name>,
) -> bool {
    if !visited.insert(current.clone()) {
        return false;
    }
    graph.get(current).is_some_and(|dependencies| {
        dependencies
            .iter()
            .any(|dependency| dependency == target || reaches(graph, dependency, target, visited))
    })
}

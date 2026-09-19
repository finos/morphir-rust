//! Alias dependency analysis. Custom types terminate alias expansion.
use super::*;

/// Find alias cycles while permitting recursion through custom types.
pub fn validate_alias_cycles(modules: &[ModuleIR]) -> Vec<ResolutionError> {
    let aliases = modules
        .iter()
        .flat_map(|module| {
            module
                .types
                .iter()
                .filter(|ty| !matches!(ty.body, TypeExpr::CustomType { .. }))
                .map(move |ty| {
                    (
                        (
                            module_name(&module.name),
                            Name::from(ty.name.as_str()).to_string(),
                        ),
                        (module, ty),
                    )
                })
        })
        .collect::<BTreeMap<_, _>>();
    let edges = aliases
        .iter()
        .map(|(key, (_, definition))| {
            let mut refs = Vec::new();
            references(&definition.body, &key.0, &mut refs);
            (key.clone(), refs)
        })
        .collect::<BTreeMap<_, _>>();
    aliases
        .iter()
        .filter(|(key, _)| reaches(key, key, &edges, &mut BTreeSet::new()))
        .map(|(_, (module, ty))| ResolutionError {
            code: "GLEAM_TYPE_CYCLE",
            module: module.name.clone(),
            span: ty.span,
            message: format!(
                "Type alias '{}' is recursive; use a custom type for recursion",
                ty.name
            ),
        })
        .collect()
}
fn references(ty: &TypeExpr, module: &str, refs: &mut Vec<(String, String)>) {
    match ty {
        TypeExpr::Resolved { name, .. } => {
            refs.push((name.module_path.to_string(), name.local_name.to_string()))
        }
        TypeExpr::Named {
            module: qualifier,
            name,
            ..
        } => refs.push((
            qualifier.as_deref().unwrap_or(module).to_owned(),
            Name::from(name.as_str()).to_string(),
        )),
        _ => {}
    }
    for child in children(ty) {
        references(child, module, refs);
    }
}
fn reaches(
    start: &(String, String),
    current: &(String, String),
    edges: &BTreeMap<(String, String), Vec<(String, String)>>,
    seen: &mut BTreeSet<(String, String)>,
) -> bool {
    if !seen.insert(current.clone()) {
        return false;
    }
    edges.get(current).is_some_and(|next| {
        next.iter()
            .any(|next| next == start || reaches(start, next, edges, seen))
    })
}

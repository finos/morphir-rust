use super::{source::Source, types::Context};
use crate::Outcome;
use morphir_core::ir::classic::{module::ModuleTypeDefinition, *};
use std::collections::{BTreeMap, HashMap, HashSet};
use syn::{ext::IdentExt, spanned::Spanned};

fn declaration(
    item: &syn::Item,
) -> Option<(
    &syn::Ident,
    &syn::Generics,
    &syn::Visibility,
    &[syn::Attribute],
)> {
    match item {
        syn::Item::Struct(s) => Some((&s.ident, &s.generics, &s.vis, &s.attrs)),
        syn::Item::Enum(e) => Some((&e.ident, &e.generics, &e.vis, &e.attrs)),
        syn::Item::Type(t) => Some((&t.ident, &t.generics, &t.vis, &t.attrs)),
        _ => None,
    }
}

pub(super) fn symbols(
    source: &Source<'_>,
    items: &[syn::Item],
) -> Outcome<BTreeMap<String, usize>> {
    let mut symbols = BTreeMap::new();
    let mut names = Vec::new();
    for item in items {
        if let Some((ident, generics, _, _)) = declaration(item) {
            names.push(ident.clone());
            let params = source.generics(generics)?;
            symbols.insert(ident.unraw().to_string(), params.len());
        } else if !matches!(item, syn::Item::Fn(_)) {
            return Err(source.error(
                item.span(),
                "RS_ITEM_UNSUPPORTED",
                "Only structs, enums and type aliases are supported",
            ));
        }
    }
    source.unique(names)?;
    let constructors = items.iter().flat_map(|item| match item {
        syn::Item::Struct(s) => vec![s.ident.clone()],
        syn::Item::Fn(function)
            if function
                .attrs
                .iter()
                .any(super::binding_attributes::is_binding_attribute) =>
        {
            vec![function.sig.ident.clone()]
        }
        syn::Item::Enum(e) => e.variants.iter().map(|v| v.ident.clone()).collect(),
        _ => vec![],
    });
    source.unique(constructors)?;
    Ok(symbols)
}

pub(super) fn lower(
    context: &Context<'_>,
    item: &syn::Item,
) -> Outcome<ModuleTypeDefinition<Attrs>> {
    let source = context.source;
    let (ident, generics, vis, attrs) = declaration(item).expect("validated declaration");
    let name = source.name(ident)?;
    let access = source.visibility(vis)?;
    let doc = source.attributes(attrs, !matches!(item, syn::Item::Type(_)))?;
    let params = source.generics(generics)?;
    let names = params
        .iter()
        .map(|p| source.name(p))
        .collect::<Outcome<Vec<_>>>()?;
    let definition = match item {
        syn::Item::Type(alias) => TypeDefinition::Alias(names, context.ty(&alias.ty, &params)?),
        syn::Item::Struct(structure) => {
            let fields = context.fields(&structure.fields, &params)?;
            let visibilities = structure
                .fields
                .iter()
                .map(|f| source.visibility(&f.vis))
                .collect::<Outcome<Vec<_>>>()?;
            if visibilities.contains(&Access::Public) && visibilities.contains(&Access::Private) {
                return Err(source.error(structure.fields.span(), "RS_VISIBILITY", "Mixed public and private fields cannot be represented without exposing private fields"));
            }
            let constructors_access = if visibilities.iter().all(|v| *v == Access::Public) {
                access.clone()
            } else {
                Access::Private
            };
            if matches!(structure.fields, syn::Fields::Named(_))
                && constructors_access == Access::Public
            {
                TypeDefinition::Alias(names, Type::Record(Attrs::None, fields))
            } else {
                let args = fields.into_iter().map(|f| (f.name, f.ty)).collect();
                TypeDefinition::Custom(
                    names,
                    AccessControlled {
                        access: constructors_access,
                        value: vec![Constructor {
                            name: name.clone(),
                            args,
                        }],
                    },
                )
            }
        }
        syn::Item::Enum(enumeration) => {
            source.unique(enumeration.variants.iter().map(|v| v.ident.clone()))?;
            let constructors = enumeration
                .variants
                .iter()
                .map(|variant| {
                    source.attributes(&variant.attrs, false)?;
                    for field in &variant.fields {
                        if !matches!(field.vis, syn::Visibility::Inherited) {
                            return Err(source.error(field.vis.span(), "RS_VISIBILITY", "Enum fields inherit visibility from the enum and cannot specify visibility"));
                        }
                    }
                    if let Some((_, expr)) = &variant.discriminant {
                        return Err(source.error(
                            expr.span(),
                            "RS_DISCRIMINANT",
                            "Enum discriminants are unsupported",
                        ));
                    }
                    let fields = context.fields(&variant.fields, &params)?;
                    Ok(Constructor {
                        name: source.name(&variant.ident)?,
                        args: fields.into_iter().map(|f| (f.name, f.ty)).collect(),
                    })
                })
                .collect::<Outcome<Vec<_>>>()?;
            TypeDefinition::Custom(
                names,
                AccessControlled {
                    access: access.clone(),
                    value: constructors,
                },
            )
        }
        _ => unreachable!("validated declaration"),
    };
    Ok((
        name,
        AccessControlled {
            access,
            value: Documented::new(doc, definition),
        },
    ))
}

pub(super) fn validate_aliases(
    context: &Context<'_>,
    items: &[syn::Item],
    definitions: &[ModuleTypeDefinition<Attrs>],
) -> Outcome<()> {
    let aliases: HashMap<_, _> = definitions
        .iter()
        .filter_map(|(name, value)| match &value.value.value {
            TypeDefinition::Alias(_, ty) => Some((name.clone(), ty)),
            _ => None,
        })
        .collect();
    let mut complete = HashSet::new();
    for item in items {
        let Some((ident, _, _, _)) = declaration(item) else {
            continue;
        };
        let name = context.source.name(ident)?;
        if aliases.contains_key(&name) {
            let mut active = HashSet::new();
            check_cycle(&name, &aliases, &mut active, &mut complete, context).map_err(|()| {
                context.source.error(
                    ident.span(),
                    "RS_ALIAS_CYCLE",
                    "Recursive type aliases are unsupported",
                )
            })?;
        }
        if let syn::Item::Type(alias) = item {
            let ty = aliases[&name];
            for param in context.source.generics(&alias.generics)? {
                if !uses_variable(ty, &context.source.name(&param)?) {
                    return Err(context.source.error(
                        param.span(),
                        "RS_UNUSED_TYPE_PARAMETER",
                        "Type alias parameter is unused",
                    ));
                }
            }
        }
    }
    Ok(())
}
fn children(ty: &Type<Attrs>) -> Vec<&Type<Attrs>> {
    match ty {
        Type::Tuple(_, args) | Type::Reference(_, _, args) => args.iter().collect(),
        Type::Record(_, fields) | Type::ExtensibleRecord(_, _, fields) => {
            fields.iter().map(|f| &f.ty).collect()
        }
        Type::Function(_, a, b) => vec![a, b],
        _ => vec![],
    }
}
fn uses_variable(ty: &Type<Attrs>, name: &Name) -> bool {
    matches!(ty, Type::Variable(_, n) if n == name)
        || children(ty).iter().any(|t| uses_variable(t, name))
}
fn check_cycle(
    name: &Name,
    aliases: &HashMap<Name, &Type<Attrs>>,
    active: &mut HashSet<Name>,
    complete: &mut HashSet<Name>,
    context: &Context<'_>,
) -> Result<(), ()> {
    if complete.contains(name) {
        return Ok(());
    }
    let Some(ty) = aliases.get(name) else {
        return Ok(());
    };
    if !active.insert(name.clone()) {
        return Err(());
    }
    fn visit(
        ty: &Type<Attrs>,
        aliases: &HashMap<Name, &Type<Attrs>>,
        active: &mut HashSet<Name>,
        complete: &mut HashSet<Name>,
        context: &Context<'_>,
    ) -> Result<(), ()> {
        if let Type::Reference(_, target, _) = ty
            && &target.package_path == context.package
            && &target.module_path == context.module
        {
            check_cycle(&target.local_name, aliases, active, complete, context)?;
        }
        for child in children(ty) {
            visit(child, aliases, active, complete, context)?;
        }
        Ok(())
    }
    visit(ty, aliases, active, complete, context)?;
    active.remove(name);
    complete.insert(name.clone());
    Ok(())
}

//! Keep executable type checks from erasing Rust storage through type aliases.
use super::*;

impl Lower<'_, '_> {
    // Type-only lowering erases Box. Executable lowering must reject it even
    // when an alias hides the storage wrapper from the function signature.
    pub(super) fn check_storage_type(&self, ty: &syn::Type) -> Outcome<()> {
        self.check_storage_type_with(ty, &self.parameters)
    }

    pub(super) fn check_storage_type_with(
        &self,
        ty: &syn::Type,
        parameters: &[syn::Ident],
    ) -> Outcome<()> {
        self.check_storage_type_inner(ty, parameters, &mut BTreeSet::new())
    }

    fn check_storage_type_inner(
        &self,
        ty: &syn::Type,
        parameters: &[syn::Ident],
        aliases: &mut BTreeSet<String>,
    ) -> Outcome<()> {
        match ty {
            syn::Type::Path(path) => {
                for segment in &path.path.segments {
                    let name = segment.ident.unraw().to_string();
                    let parameter = parameters.iter().any(|p| p.unraw() == name);
                    if name == "Box" && !parameter && !self.context.symbols.contains_key("Box") {
                        return Err(self.error(
                            ty,
                            "Box types are not supported in executable types or pattern payloads",
                        ));
                    }
                    if let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments {
                        for argument in &arguments.args {
                            if let syn::GenericArgument::Type(ty) = argument {
                                self.check_storage_type_inner(ty, parameters, aliases)?;
                            }
                        }
                    }
                    if !parameter && path.qself.is_none() && path.path.segments.len() == 1 {
                        let alias = self.context.items.iter().find_map(|item| match item {
                            syn::Item::Type(alias) if alias.ident.unraw() == name => Some(alias),
                            _ => None,
                        });
                        if let Some(alias) = alias {
                            if !aliases.insert(name.clone()) {
                                return Err(
                                    self.error(ty, "Recursive type aliases are unsupported")
                                );
                            }
                            let alias_parameters = self.context.source.generics(&alias.generics)?;
                            self.check_storage_type_inner(&alias.ty, &alias_parameters, aliases)?;
                            aliases.remove(&name);
                        }
                    }
                }
            }
            syn::Type::Tuple(tuple) => {
                for ty in &tuple.elems {
                    self.check_storage_type_inner(ty, parameters, aliases)?;
                }
            }
            syn::Type::Paren(paren) => {
                self.check_storage_type_inner(&paren.elem, parameters, aliases)?
            }
            syn::Type::Group(group) => {
                self.check_storage_type_inner(&group.elem, parameters, aliases)?
            }
            _ => {}
        }
        Ok(())
    }
}

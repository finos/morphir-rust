use crate::{Outcome, error};
use morphir_core::naming::{Name, Path};
use proc_macro2::{Ident, TokenStream};
use quote::quote;
use std::collections::BTreeSet;

pub(super) fn ident(text: &str) -> Outcome<Ident> {
    if text == "_" {
        return Err(error("RS_NAME", "A Rust identifier cannot be '_'"));
    }
    syn::parse_str::<Ident>(text)
        .or_else(|_| syn::parse_str::<Ident>(&format!("r#{text}")))
        .map_err(|_| {
            error(
                "RS_NAME",
                format!("Cannot represent {text:?} as a Rust identifier"),
            )
        })
}

pub(super) fn type_name(name: &Name) -> Outcome<Ident> {
    ident(&name.to_pascal_case())
}
pub(super) fn field_name(name: &Name) -> Outcome<Ident> {
    ident(&name.to_snake_case())
}

pub(super) fn module_names(module: &str) -> Outcome<Vec<Ident>> {
    Path::from_canonical_string(module)
        .map_err(|e| error("RS_NAME", e))?
        .segments
        .iter()
        .map(field_name)
        .collect()
}

pub(super) fn reserve(names: &mut BTreeSet<String>, name: &Ident) -> Outcome<()> {
    if !names.insert(name.to_string()) {
        return Err(error("RS_NAME", format!("Rust name collision: {name}")));
    }
    Ok(())
}

pub(super) fn parameters(params: &[Name]) -> Outcome<Vec<Ident>> {
    let mut seen = BTreeSet::new();
    params
        .iter()
        .map(|p| {
            let id = type_name(p)?;
            reserve(&mut seen, &id)?;
            Ok(id)
        })
        .collect()
}

pub(super) fn generics(params: &[Ident]) -> TokenStream {
    if params.is_empty() {
        quote!()
    } else {
        quote!(<#(#params),*>)
    }
}

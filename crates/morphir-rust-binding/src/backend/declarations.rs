use super::{
    ir::{Body, Declaration},
    names::*,
    types::{Renderer, marker_field, variables},
};
use crate::{Outcome, error};
use morphir_core::ir::v4::{Access, Type};
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::BTreeSet;

pub(super) fn visibility(access: Access) -> TokenStream {
    match access {
        Access::Public => quote!(pub),
        Access::Private => quote!(pub(crate)),
    }
}

pub(super) fn render(
    renderer: &mut Renderer<'_>,
    declaration: &Declaration,
) -> Outcome<TokenStream> {
    let name = type_name(&declaration.name)?;
    let params = parameters(&declaration.params)?;
    let generic = generics(&params);
    let vis = visibility(declaration.access);
    let doc = declaration
        .doc
        .as_ref()
        .filter(|d| !d.is_empty())
        .map(|d| quote!(#[doc = #d]));
    let result = match &declaration.body {
        Body::Alias(Type::Record(_, fields))
        | Body::Alias(Type::ExtensibleRecord(_, _, fields)) => {
            let row = if let Body::Alias(Type::ExtensibleRecord(_, row, _)) = &declaration.body {
                Some(row)
            } else {
                None
            };
            let types: Vec<_> = fields.iter().map(|f| &f.tpe).collect();
            let marker = marker_field(&renderer.markers(declaration, &types, row)?);
            let fields = renderer.fields(fields, row, declaration)?;
            quote!(#vis struct #name #generic { #(#fields,)* #marker })
        }
        Body::Alias(tpe) => {
            let mut used = BTreeSet::new();
            variables(tpe, &mut used);
            let tpe = renderer.expression(tpe, declaration)?;
            if declaration
                .params
                .iter()
                .any(|p| !used.contains(&p.to_canonical_string()))
            {
                // Rust rejects unused alias parameters (E0091). An associated type keeps
                // the alias transparent without adding a wrapper or changing inhabitants.
                let helper = ident(&format!("__MorphirAlias{name}"))?;
                reserve(
                    renderer
                        .names
                        .entry(declaration.module.clone())
                        .or_default(),
                    &helper,
                )?;
                quote!(
                    #[doc(hidden)] #vis trait #helper #generic { type Output; }
                    impl #generic #helper #generic for () { type Output = #tpe; }
                    #vis type #name #generic = <() as #helper #generic>::Output;
                )
            } else {
                quote!(#vis type #name #generic = #tpe;)
            }
        }
        Body::Opaque => {
            let path = renderer.external.get(&declaration.fqname).ok_or_else(|| {
                error(
                    "RS_OPAQUE",
                    format!(
                        "Opaque type {} needs an externalTypes binding",
                        declaration.fqname
                    ),
                )
            })?;
            quote!(#vis type #name #generic = #path #generic;)
        }
        Body::Derived(tpe, from, to) => {
            let tpe = renderer.expression(tpe, declaration)?;
            let conversion = format!(
                "Derived representation. Morphir conversions: {from} and {to}. Conversion bodies are not generated."
            );
            let marker = if params.is_empty() {
                quote!()
            } else {
                quote!(, ::std::marker::PhantomData<(#(#params,)*)>)
            };
            quote!(#[doc = #conversion] #vis struct #name #generic (#tpe #marker);)
        }
        Body::Custom(access, constructors) => {
            let types: Vec<_> = constructors
                .iter()
                .flat_map(|c| c.args.iter().map(|a| &a.arg_type))
                .collect();
            let markers = renderer.markers(declaration, &types, None)?;
            let mut seen = BTreeSet::new();
            let mut variants = vec![];
            for constructor in constructors {
                let variant = type_name(&constructor.name)?;
                reserve(&mut seen, &variant)?;
                let mut fields = constructor
                    .args
                    .iter()
                    .map(|a| renderer.expression(&a.arg_type, declaration))
                    .collect::<Outcome<Vec<_>>>()?;
                if !markers.is_empty() {
                    fields.push(quote!(::std::marker::PhantomData<(#(#markers,)*)>));
                }
                variants.push(if fields.is_empty() {
                    quote!(#variant)
                } else {
                    quote!(#variant(#(#fields),*))
                });
            }
            let enum_body = if constructors.is_empty() && !params.is_empty() {
                quote!(#vis struct #name #generic { __never: ::std::convert::Infallible, __marker: ::std::marker::PhantomData<(#(#params,)*)> })
            } else {
                quote!(#vis enum #name #generic { #(#variants,)* })
            };
            if *access == Access::Public {
                enum_body
            } else {
                let storage = ident(&format!(
                    "__morphir_private_{}",
                    declaration.name.to_snake_case()
                ))?;
                reserve(
                    renderer
                        .names
                        .entry(declaration.module.clone())
                        .or_default(),
                    &storage,
                )?;
                let repr = if constructors.is_empty() && !params.is_empty() {
                    quote!(pub(crate) struct Repr #generic { __never: ::std::convert::Infallible, __marker: ::std::marker::PhantomData<(#(#params,)*)> })
                } else {
                    quote!(pub(crate) enum Repr #generic { #(#variants,)* })
                };
                quote!(mod #storage { #repr } #vis struct #name #generic (#storage::Repr #generic);)
            }
        }
    };
    Ok(quote!(#doc #result))
}

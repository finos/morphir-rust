use super::{
    ir::{Body, Declaration, Package},
    names::*,
};
use crate::{Outcome, error};
use morphir_core::{
    ir::v4::{Field, Type},
    naming::Name,
};
use proc_macro2::{Ident, TokenStream};
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Symbol {
    pub path: TokenStream,
    pub arity: usize,
}

pub(super) struct Renderer<'a> {
    pub package: &'a Package,
    pub symbols: BTreeMap<String, Symbol>,
    pub external: BTreeMap<String, syn::Path>,
    pub names: BTreeMap<String, BTreeSet<String>>,
    pub helpers: BTreeMap<String, Vec<TokenStream>>,
}

#[derive(Clone, Copy)]
enum RenderContext {
    TypeDefinition,
    FunctionSignature,
}

impl Renderer<'_> {
    pub fn markers(
        &self,
        owner: &Declaration,
        types: &[&Type],
        row: Option<&Name>,
    ) -> Outcome<Vec<Ident>> {
        fn used(tpe: &Type, owner: &Declaration, package: &Package, vars: &mut BTreeSet<String>) {
            match tpe {
                Type::Reference(_, name, _)
                    if recursive(
                        package,
                        &owner.fqname,
                        &name.to_canonical_string(),
                        &mut BTreeSet::new(),
                    ) => {}
                Type::Reference(_, _, args) | Type::Tuple(_, args) => {
                    for arg in args {
                        used(arg, owner, package, vars);
                    }
                }
                Type::Record(_, fields) | Type::ExtensibleRecord(_, _, fields) => {
                    if let Type::ExtensibleRecord(_, row, _) = tpe {
                        vars.insert(row.to_canonical_string());
                    }
                    for field in fields {
                        used(&field.tpe, owner, package, vars);
                    }
                }
                Type::Function(_, a, b) => {
                    used(a, owner, package, vars);
                    used(b, owner, package, vars);
                }
                _ => variables(tpe, vars),
            }
        }
        let mut vars = BTreeSet::new();
        for tpe in types {
            used(tpe, owner, self.package, &mut vars);
        }
        if let Some(row) = row {
            vars.insert(row.to_canonical_string());
        }
        owner
            .params
            .iter()
            .filter(|p| !vars.contains(&p.to_canonical_string()))
            .map(type_name)
            .collect()
    }

    pub fn expression(&mut self, tpe: &Type, owner: &Declaration) -> Outcome<TokenStream> {
        self.expression_in(tpe, owner, RenderContext::TypeDefinition)
    }

    pub fn signature(&mut self, tpe: &Type, owner: &Declaration) -> Outcome<TokenStream> {
        self.expression_in(tpe, owner, RenderContext::FunctionSignature)
    }

    fn expression_in(
        &mut self,
        tpe: &Type,
        owner: &Declaration,
        context: RenderContext,
    ) -> Outcome<TokenStream> {
        match tpe {
            Type::Unit(_) => Ok(quote!(())),
            Type::Variable(_, name) => {
                if !owner.params.contains(name) {
                    return Err(error(
                        "RS_TYPE",
                        format!("Unbound type variable {name} in {}", owner.fqname),
                    ));
                }
                let name = type_name(name)?;
                Ok(quote!(#name))
            }
            Type::Tuple(_, items) => {
                let items = items
                    .iter()
                    .map(|t| self.expression_in(t, owner, context))
                    .collect::<Outcome<Vec<_>>>()?;
                Ok(quote!((#(#items,)*)))
            }
            Type::Function(_, input, output) => {
                let input = self.expression_in(input, owner, context)?;
                let output = self.expression_in(output, owner, context)?;
                Ok(quote!(::std::rc::Rc<dyn ::std::ops::Fn(#input) -> #output>))
            }
            Type::Record(_, fields) => self.record_helper(fields, None, owner),
            Type::ExtensibleRecord(_, row, fields) => self.record_helper(fields, Some(row), owner),
            Type::Reference(_, name, args) => {
                let fqname = name.to_canonical_string();
                let args = args
                    .iter()
                    .map(|t| self.expression_in(t, owner, context))
                    .collect::<Outcome<Vec<_>>>()?;
                if let Some(symbol) = self.symbols.get(&fqname) {
                    arity(&fqname, symbol.arity, args.len())?;
                    let path = &symbol.path;
                    let generics = arguments(&args);
                    let tpe = quote!(#path #generics);
                    return Ok(
                        if matches!(context, RenderContext::TypeDefinition)
                            && recursive(self.package, &owner.fqname, &fqname, &mut BTreeSet::new())
                        {
                            quote!(::std::boxed::Box<#tpe>)
                        } else {
                            tpe
                        },
                    );
                }
                if let Some(path) = self.external.get(&fqname) {
                    if let Some(expected) = self.package.dependencies.get(&fqname) {
                        arity(&fqname, *expected, args.len())?;
                    }
                    let generics = arguments(&args);
                    return Ok(quote!(#path #generics));
                }
                if let Some((path, expected)) = sdk(&fqname) {
                    arity(&fqname, expected, args.len())?;
                    let args = if fqname == "morphir/SDK:result#result" {
                        args.into_iter().rev().collect()
                    } else {
                        args
                    };
                    let generics = arguments(&args);
                    return Ok(quote!(#path #generics));
                }
                Err(error(
                    "RS_REFERENCE",
                    format!(
                        "Unresolved type {fqname}; supply an externalTypes binding for dependency types"
                    ),
                ))
            }
        }
    }

    pub fn fields(
        &mut self,
        fields: &[Field],
        row: Option<&Name>,
        owner: &Declaration,
    ) -> Outcome<Vec<TokenStream>> {
        let mut names = BTreeSet::new();
        let mut result = vec![];
        for field in fields {
            let name = field_name(&field.name)?;
            reserve(&mut names, &name)?;
            let tpe = self.expression(&field.tpe, owner)?;
            result.push(quote!(pub #name: #tpe));
        }
        if let Some(row) = row {
            let name = ident("remaining_fields")?;
            reserve(&mut names, &name)?;
            if !owner.params.contains(row) {
                return Err(error("RS_TYPE", format!("Unbound row variable {row}")));
            }
            let row = type_name(row)?;
            result.push(quote!(pub #name: #row));
        }
        Ok(result)
    }

    fn record_helper(
        &mut self,
        fields: &[Field],
        row: Option<&Name>,
        owner: &Declaration,
    ) -> Outcome<TokenStream> {
        let names = self.names.entry(owner.module.clone()).or_default();
        let mut sequence = 0;
        let id = loop {
            let id = ident(&format!("__MorphirRecord{sequence}"))?;
            if names.insert(id.to_string()) {
                break id;
            }
            sequence += 1;
        };
        let mut used = BTreeSet::new();
        for field in fields {
            variables(&field.tpe, &mut used);
        }
        if let Some(row) = row {
            used.insert(row.to_canonical_string());
        }
        let params: Vec<_> = owner
            .params
            .iter()
            .filter(|p| used.contains(&p.to_canonical_string()))
            .cloned()
            .collect();
        let params = parameters(&params)?;
        let generic = generics(&params);
        let types: Vec<_> = fields.iter().map(|f| &f.tpe).collect();
        let markers = self
            .markers(owner, &types, row)?
            .into_iter()
            .filter(|p| params.contains(p))
            .collect::<Vec<_>>();
        let marker = marker_field(&markers);
        let fields = self.fields(fields, row, owner)?;
        self.helpers
            .entry(owner.module.clone())
            .or_default()
            .push(quote!(
                #[doc = "Generated structural record representation."]
                pub struct #id #generic { #(#fields,)* #marker }
            ));
        let module = module_names(&owner.module)?;
        Ok(quote!(crate::#(#module::)* #id #generic))
    }
}

pub(super) fn marker_field(params: &[Ident]) -> TokenStream {
    if params.is_empty() {
        quote!()
    } else {
        quote!(#[doc = "Preserves type parameters in recursive and phantom representations."] pub __morphir_marker: ::std::marker::PhantomData<(#(#params,)*)>,)
    }
}

pub(super) fn arguments(args: &[TokenStream]) -> TokenStream {
    if args.is_empty() {
        quote!()
    } else {
        quote!(<#(#args),*>)
    }
}

pub(super) fn arity(name: &str, expected: usize, found: usize) -> Outcome<()> {
    if expected != found {
        Err(error(
            "RS_TYPE",
            format!("{name} expects {expected} type arguments, got {found}"),
        ))
    } else {
        Ok(())
    }
}

pub(super) fn variables(tpe: &Type, output: &mut BTreeSet<String>) {
    match tpe {
        Type::Variable(_, name) => {
            output.insert(name.to_canonical_string());
        }
        Type::ExtensibleRecord(_, row, fields) => {
            output.insert(row.to_canonical_string());
            for field in fields {
                variables(&field.tpe, output);
            }
        }
        Type::Record(_, fields) => {
            for field in fields {
                variables(&field.tpe, output);
            }
        }
        Type::Tuple(_, items) | Type::Reference(_, _, items) => {
            for item in items {
                variables(item, output);
            }
        }
        Type::Function(_, a, b) => {
            variables(a, output);
            variables(b, output);
        }
        Type::Unit(_) => (),
    }
}

fn sdk(name: &str) -> Option<(TokenStream, usize)> {
    Some(match name {
        "morphir/SDK:basics#bool" => (quote!(bool), 0),
        "morphir/SDK:basics#int" => (quote!(i64), 0),
        "morphir/SDK:basics#float" => (quote!(f64), 0),
        "morphir/SDK:basics#order" => (quote!(::std::cmp::Ordering), 0),
        "morphir/SDK:basics#never" => (quote!(::std::convert::Infallible), 0),
        "morphir/SDK:char#char" => (quote!(char), 0),
        "morphir/SDK:string#string" => (quote!(::std::string::String), 0),
        "morphir/SDK:list#list" => (quote!(::std::vec::Vec), 1),
        "morphir/SDK:maybe#maybe" => (quote!(::std::option::Option), 1),
        // Morphir Result takes error first; handled by the caller below.
        "morphir/SDK:result#result" => (quote!(::std::result::Result), 2),
        "morphir/SDK:dict#dict" => (quote!(::std::collections::BTreeMap), 2),
        "morphir/SDK:set#set" => (quote!(::std::collections::BTreeSet), 1),
        _ => return None,
    })
}

pub(super) fn references(tpe: &Type, output: &mut BTreeSet<String>, stop_at_record: bool) {
    match tpe {
        Type::Reference(_, name, args) => {
            output.insert(name.to_canonical_string());
            for arg in args {
                references(arg, output, stop_at_record);
            }
        }
        Type::Tuple(_, items) => {
            for item in items {
                references(item, output, stop_at_record);
            }
        }
        Type::Function(_, a, b) => {
            references(a, output, stop_at_record);
            references(b, output, stop_at_record);
        }
        Type::Record(_, fields) | Type::ExtensibleRecord(_, _, fields) if !stop_at_record => {
            for f in fields {
                references(&f.tpe, output, stop_at_record);
            }
        }
        _ => (),
    }
}

fn recursive(package: &Package, owner: &str, target: &str, visited: &mut BTreeSet<String>) -> bool {
    if owner == target {
        return true;
    }
    if !visited.insert(target.into()) {
        return false;
    }
    let Some(declaration) = package.declarations.iter().find(|d| d.fqname == target) else {
        return false;
    };
    let mut next = BTreeSet::new();
    match &declaration.body {
        Body::Alias(t) | Body::Derived(t, ..) => references(t, &mut next, false),
        Body::Custom(_, constructors) => {
            for c in constructors {
                for a in &c.args {
                    references(&a.arg_type, &mut next, false);
                }
            }
        }
        Body::Opaque => (),
    }
    next.iter()
        .any(|next| recursive(package, owner, next, visited))
}

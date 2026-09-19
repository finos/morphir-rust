use super::source::Source;
use crate::Outcome;
use morphir_core::ir::classic::*;
use std::collections::BTreeMap;
use syn::{ext::IdentExt, spanned::Spanned};

pub(super) struct Context<'a> {
    pub items: &'a [syn::Item],
    pub source: &'a Source<'a>,
    pub symbols: &'a BTreeMap<String, usize>,
    pub package: &'a Path,
    pub module: &'a Path,
}
impl Context<'_> {
    pub fn ty(&self, ty: &syn::Type, parameters: &[syn::Ident]) -> Outcome<Type<Attrs>> {
        match ty {
            syn::Type::FnPtr(function)
                if function.unsafety.is_none()
                    && function.abi.is_none()
                    && function.lifetimes.is_none()
                    && function.variadic.is_none() =>
            {
                let mut inputs = function
                    .inputs
                    .iter()
                    .map(|i| self.ty(&i.ty, parameters))
                    .collect::<Outcome<Vec<_>>>()?;
                if inputs.is_empty() {
                    inputs.push(Type::Unit(Attrs::None));
                }
                let output = match &function.output {
                    syn::ReturnType::Default => Type::Unit(Attrs::None),
                    syn::ReturnType::Type(_, t) => self.ty(t, parameters)?,
                };
                Ok(inputs.iter().rev().fold(output, |output, input| {
                    Type::Function(Attrs::None, Box::new(input.clone()), Box::new(output))
                }))
            }

            syn::Type::Tuple(tuple) if tuple.elems.is_empty() => Ok(Type::Unit(Attrs::None)),
            syn::Type::Tuple(tuple) => Ok(Type::Tuple(
                Attrs::None,
                tuple
                    .elems
                    .iter()
                    .map(|t| self.ty(t, parameters))
                    .collect::<Outcome<_>>()?,
            )),
            syn::Type::Paren(paren) => self.ty(&paren.elem, parameters),
            syn::Type::Group(group) => self.ty(&group.elem, parameters),
            syn::Type::Path(path)
                if path.qself.is_none()
                    && path.path.leading_colon.is_none()
                    && path.path.segments.len() == 1 =>
            {
                let segment = &path.path.segments[0];
                let text = segment.ident.unraw().to_string();
                let args = match &segment.arguments {
                    syn::PathArguments::None => vec![],
                    syn::PathArguments::AngleBracketed(args) if args.colon2_token.is_none() => args
                        .args
                        .iter()
                        .map(|arg| match arg {
                            syn::GenericArgument::Type(t) => self.ty(t, parameters),
                            _ => Err(self.source.error(
                                arg.span(),
                                "RS_TYPE_ARGUMENT",
                                "Only type arguments are supported",
                            )),
                        })
                        .collect::<Outcome<Vec<_>>>()?,
                    _ => {
                        return Err(self.source.error(
                            segment.arguments.span(),
                            "RS_TYPE_ARGUMENT",
                            "Unsupported generic arguments",
                        ));
                    }
                };
                if parameters
                    .iter()
                    .any(|p| p.unraw() == segment.ident.unraw())
                {
                    self.arity(ty, args.len(), 0)?;
                    return Ok(Type::Variable(
                        Attrs::None,
                        self.source.name(&segment.ident)?,
                    ));
                }
                if let Some(arity) = self.symbols.get(&text) {
                    self.arity(ty, args.len(), *arity)?;
                    return Ok(Type::Reference(
                        Attrs::None,
                        FQName::new(
                            self.package.clone(),
                            self.module.clone(),
                            self.source.name(&segment.ident)?,
                        ),
                        args,
                    ));
                }
                let (module, name, arity) = match text.as_str() {
                    "bool" => ("Basics", "Bool", 0),
                    "char" => ("Char", "Char", 0),
                    "String" => ("String", "String", 0),
                    "i64" => ("Basics", "Int", 0),
                    "f64" => ("Basics", "Float", 0),
                    "Vec" => ("List", "List", 1),
                    "Option" => ("Maybe", "Maybe", 1),
                    "Result" => ("Result", "Result", 2),
                    "Box" => {
                        self.arity(ty, args.len(), 1)?;
                        return Ok(args.into_iter().next().expect("checked arity"));
                    }
                    _ => {
                        return Err(self.source.error(
                            segment.ident.span(),
                            "RS_UNKNOWN_TYPE",
                            format!("Unknown or unbound type {text}"),
                        ));
                    }
                };
                self.arity(ty, args.len(), arity)?;
                // Rust Result<T, E> and Morphir Result<E, T> have opposite parameter order.
                let args = if text == "Result" {
                    args.into_iter().rev().collect()
                } else {
                    args
                };
                Ok(Type::Reference(
                    Attrs::None,
                    FQName::new(
                        Path::new(vec!["Morphir".parse().unwrap(), "SDK".parse().unwrap()]),
                        Path::new(vec![module.parse().unwrap()]),
                        name.parse().unwrap(),
                    ),
                    args,
                ))
            }
            _ => Err(self.source.error(
                ty.span(),
                "RS_TYPE_UNSUPPORTED",
                "Unsupported Rust type syntax",
            )),
        }
    }
    fn arity(&self, ty: &syn::Type, actual: usize, expected: usize) -> Outcome<()> {
        if actual == expected {
            Ok(())
        } else {
            Err(self.source.error(
                ty.span(),
                "RS_TYPE_ARITY",
                format!("Expected {expected} type arguments, got {actual}"),
            ))
        }
    }
    pub fn fields(
        &self,
        fields: &syn::Fields,
        params: &[syn::Ident],
    ) -> Outcome<Vec<Field<Attrs>>> {
        let names = fields
            .iter()
            .enumerate()
            .map(|(i, f)| {
                f.ident
                    .clone()
                    .unwrap_or_else(|| syn::Ident::new(&format!("field{i}"), f.span()))
            })
            .collect::<Vec<_>>();
        let names = self.source.unique(names)?;
        fields
            .iter()
            .zip(names)
            .map(|(field, name)| {
                self.source.attributes(&field.attrs, false)?;
                self.source.visibility(&field.vis)?;
                Ok(Field {
                    name,
                    ty: self.ty(&field.ty, params)?,
                })
            })
            .collect()
    }
}

use super::{declarations::visibility, ir::Function, names::*, types::Renderer};
use crate::{Outcome, error, values};
use morphir_core::{
    ir::v4::{Literal, Type, Value, ValueBody},
    naming::Name,
};
use proc_macro2::{Ident, TokenStream};
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
struct Binding {
    name: Ident,
    copy: bool,
    id: usize,
}
type Scope = BTreeMap<String, Binding>;

pub(super) fn render(renderer: &mut Renderer<'_>, function: &Function) -> Outcome<TokenStream> {
    values::validate_function(&function.definition)
        .map_err(|e| error("RS_VALUE", format!("{}: {e}", function.owner.fqname)))?;
    for tpe in function
        .definition
        .input_types
        .values()
        .chain(function.definition.output_type.iter())
    {
        validate_signature(tpe)?;
    }
    let owner = &function.owner;

    let name = field_name(&owner.name)?;
    let generic = generics(&parameters(&owner.params)?);
    let vis = visibility(owner.access);
    let doc = owner
        .doc
        .as_ref()
        .filter(|d| !d.is_empty())
        .map(|d| quote!(#[doc = #d]));
    let mut scope = Scope::new();
    let mut seen = BTreeSet::new();
    let mut next = 0;
    let mut parameters = vec![];
    for (text, tpe) in &function.definition.input_types {
        let name =
            field_name(&Name::from_canonical_string(text).map_err(|e| error("RS_NAME", e))?)?;
        reserve(&mut seen, &name)?;
        scope.insert(
            text.clone(),
            Binding {
                name: name.clone(),
                copy: is_copy(tpe),
                id: next,
            },
        );
        next += 1;
        let tpe = renderer.signature(tpe, owner)?;
        parameters.push(quote!(#name: #tpe));
    }
    let output = renderer.signature(
        function
            .definition
            .output_type
            .as_ref()
            .expect("validated output"),
        owner,
    )?;
    let ValueBody::Expression(body) = &function.definition.body else {
        unreachable!()
    };
    let body = expression(body, &scope, &mut BTreeSet::new(), &mut next, false)?;
    Ok(quote!(#doc #vis fn #name #generic (#(#parameters),*) -> #output { #body }))
}

fn validate_signature(tpe: &Type) -> Outcome<()> {
    match tpe {
        Type::Record(..) | Type::ExtensibleRecord(..) | Type::Function(..) => Err(error(
            "RS_VALUE",
            "Function signatures require named types; structural records and higher-order functions are unsupported",
        )),
        Type::Tuple(_, elements) | Type::Reference(_, _, elements) => {
            for element in elements {
                validate_signature(element)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn is_copy(tpe: &Type) -> bool {
    match tpe {
        Type::Unit(_) => true,
        Type::Tuple(_, fields) => fields.iter().all(is_copy),
        Type::Reference(_, name, args) if args.is_empty() => matches!(
            name.to_canonical_string().as_str(),
            "morphir/SDK:basics#int"
                | "morphir/SDK:basics#float"
                | "morphir/SDK:basics#bool"
                | "morphir/SDK:char#char"
        ),
        _ => false,
    }
}

fn expression(
    value: &Value,
    scope: &Scope,
    moved: &mut BTreeSet<usize>,
    next: &mut usize,
    borrow: bool,
) -> Outcome<TokenStream> {
    Ok(match value {
        Value::Unit(_) => quote!(()),
        Value::Literal(_, literal) => match literal {
            Literal::Bool(v) => quote!(#v),
            Literal::Char(v) => quote!(#v),
            Literal::Integer(v) => {
                let v = v
                    .to_string()
                    .parse::<i64>()
                    .map_err(|_| error("RS_VALUE", "Integer outside i64 range"))?;
                quote!(#v)
            }
            Literal::Float(v) => {
                let v = v.value();
                quote!(#v)
            }
            Literal::String(v) => quote!(::std::string::String::from(#v)),
            _ => return Err(error("RS_VALUE", "Unsupported literal")),
        },
        Value::Variable(_, name) => {
            let binding = scope
                .get(&name.to_canonical_string())
                .ok_or_else(|| error("RS_VALUE", format!("Unknown variable {name}")))?;
            if moved.contains(&binding.id) {
                return Err(error(
                    "RS_VALUE",
                    format!("Non-Copy value {name} is used after it was moved"),
                ));
            }
            if !borrow && !binding.copy {
                moved.insert(binding.id);
            }
            let name = &binding.name;
            quote!(#name)
        }
        Value::Tuple(_, fields) => {
            let fields = fields
                .iter()
                .map(|f| expression(f, scope, moved, next, false))
                .collect::<Outcome<Vec<_>>>()?;
            quote!((#(#fields,)*))
        }
        Value::IfThenElse(_, condition, yes, no) => {
            let condition = expression(condition, scope, moved, next, false)?;
            let mut yes_moves = moved.clone();
            let mut no_moves = moved.clone();
            let yes = expression(yes, scope, &mut yes_moves, next, false)?;
            let no = expression(no, scope, &mut no_moves, next, false)?;
            moved.extend(yes_moves);
            moved.extend(no_moves);
            quote!(if #condition { #yes } else { #no })
        }
        Value::LetDefinition(_, name, definition, continuation) => {
            let ValueBody::Expression(body) = &definition.body else {
                return Err(error("RS_VALUE", "Local expression required"));
            };
            let body = expression(body, scope, moved, next, false)?;
            let rust_name = field_name(name)?;
            let canonical = name.to_canonical_string();
            if scope
                .iter()
                .any(|(n, b)| n != &canonical && b.name == rust_name)
            {
                return Err(error("RS_NAME", format!("Local name collision: {name}")));
            }
            let mut scope = scope.clone();
            scope.insert(
                canonical,
                Binding {
                    name: rust_name.clone(),
                    copy: is_copy(
                        definition
                            .output_type
                            .as_ref()
                            .expect("validated local type"),
                    ),
                    id: *next,
                },
            );
            *next += 1;
            let continuation = expression(continuation, &scope, moved, next, false)?;
            quote!({ let #rust_name = #body; #continuation })
        }
        Value::Apply(..) => {
            let (operator, left, right) =
                values::comparison(value).map_err(|e| error("RS_VALUE", e))?;
            let borrowed = if let Value::Variable(_, name) = left {
                scope
                    .get(&name.to_canonical_string())
                    .filter(|binding| !binding.copy)
                    .map(|binding| binding.id)
            } else {
                None
            };
            let left = expression(left, scope, moved, next, true)?;

            let right = expression(right, scope, moved, next, true)?;
            if borrowed.is_some_and(|id| moved.contains(&id)) {
                return Err(error(
                    "RS_VALUE",
                    "Comparison moves its borrowed left operand",
                ));
            }
            let operator: TokenStream = operator.symbol().parse().expect("known operator");

            quote!((#left) #operator (#right))
        }
        _ => return Err(error("RS_VALUE", "Unsupported Rust value expression")),
    })
}

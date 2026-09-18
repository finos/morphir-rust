use super::{
    declarations::visibility,
    ir::{Body, Function},
    names::*,
    types::Renderer,
};
use crate::{Outcome, error, values};
use morphir_core::{
    ir::v4::{Access, Literal, Pattern, Type, Value, ValueBody},
    naming::{FQName, Name},
};
use proc_macro2::{Ident, TokenStream};
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
struct Binding {
    name: Ident,
    copy: bool,
    tpe: Type,
    id: usize,
}
type Scope = BTreeMap<String, Binding>;

pub(super) fn render(renderer: &mut Renderer<'_>, function: &Function) -> Outcome<TokenStream> {
    let context = pattern_context(renderer)?;
    values::validate_function(&function.definition, &context)
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
                tpe: tpe.clone(),
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
    let body = expression(
        body,
        &scope,
        &mut BTreeSet::new(),
        &mut next,
        false,
        renderer,
        &context,
    )?;
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
    renderer: &Renderer<'_>,
    context: &crate::patterns::Context,
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
                .map(|f| expression(f, scope, moved, next, false, renderer, context))
                .collect::<Outcome<Vec<_>>>()?;
            quote!((#(#fields,)*))
        }
        Value::IfThenElse(_, condition, yes, no) => {
            let condition = expression(condition, scope, moved, next, false, renderer, context)?;
            let mut yes_moves = moved.clone();
            let mut no_moves = moved.clone();
            let yes = expression(yes, scope, &mut yes_moves, next, false, renderer, context)?;
            let no = expression(no, scope, &mut no_moves, next, false, renderer, context)?;
            moved.extend(yes_moves);
            moved.extend(no_moves);
            quote!(if #condition { #yes } else { #no })
        }
        Value::PatternMatch(_, subject, cases) => {
            let environment = scope
                .iter()
                .map(|(name, binding)| (name.clone(), binding.tpe.clone()))
                .collect();
            let subject_type =
                values::infer(subject, &environment, context).map_err(|e| error("RS_VALUE", e))?;
            let subject = expression(subject, scope, moved, next, false, renderer, context)?;
            let mut arms = vec![];
            let mut all_moves = moved.clone();
            for case in cases {
                let bindings = crate::patterns::bindings(&case.0, &subject_type, context)
                    .map_err(|e| error("RS_VALUE", e))?;
                let mut arm_scope = scope.clone();
                for (name, tpe) in bindings {
                    let rust_name = loop {
                        let candidate = ident(&format!("__morphir_binding_{}", *next))?;
                        *next += 1;
                        if !arm_scope.values().any(|binding| binding.name == candidate) {
                            break candidate;
                        }
                    };
                    arm_scope.insert(
                        name,
                        Binding {
                            name: rust_name,
                            copy: is_copy(&tpe),
                            tpe,
                            id: *next,
                        },
                    );
                    *next += 1;
                }
                let pattern = pattern(&case.0, &subject_type, &arm_scope, renderer, context)?;
                let mut arm_moves = moved.clone();
                let body = expression(
                    &case.1,
                    &arm_scope,
                    &mut arm_moves,
                    next,
                    false,
                    renderer,
                    context,
                )?;
                all_moves.extend(arm_moves);
                arms.push(quote!(#pattern => { #body }));
            }
            moved.extend(all_moves);
            quote!(match #subject { #(#arms,)* })
        }
        Value::LetDefinition(_, name, definition, continuation) => {
            let ValueBody::Expression(body) = &definition.body else {
                return Err(error("RS_VALUE", "Local expression required"));
            };
            let body = expression(body, scope, moved, next, false, renderer, context)?;
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
                    tpe: definition
                        .output_type
                        .clone()
                        .expect("validated local type"),
                    id: *next,
                },
            );
            *next += 1;
            let continuation =
                expression(continuation, &scope, moved, next, false, renderer, context)?;
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
            let left = expression(left, scope, moved, next, true, renderer, context)?;

            let right = expression(right, scope, moved, next, true, renderer, context)?;
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

fn pattern_context(renderer: &Renderer<'_>) -> Outcome<crate::patterns::Context> {
    let mut context = crate::patterns::Context::default();
    for declaration in &renderer.package.declarations {
        if let Body::Custom(_, constructors) = &declaration.body {
            let prefix = declaration
                .fqname
                .rsplit_once('#')
                .expect("declaration FQName")
                .0;
            context.types.insert(
                declaration.fqname.clone(),
                crate::patterns::CustomType {
                    parameters: declaration.params.clone(),
                    constructors: constructors
                        .iter()
                        .map(|constructor| {
                            Ok(crate::patterns::Constructor {
                                name: FQName::from_canonical_string(&format!(
                                    "{prefix}#{}",
                                    constructor.name.to_canonical_string()
                                ))
                                .map_err(|e| error("RS_NAME", e))?,
                                arguments: constructor
                                    .args
                                    .iter()
                                    .map(|arg| arg.arg_type.clone())
                                    .collect(),
                            })
                        })
                        .collect::<Outcome<_>>()?,
                },
            );
        }
    }
    Ok(context)
}

fn pattern(
    value: &Pattern,
    subject: &Type,
    scope: &Scope,
    renderer: &Renderer<'_>,
    context: &crate::patterns::Context,
) -> Outcome<TokenStream> {
    Ok(match value {
        Pattern::WildcardPattern(_) => quote!(_),
        Pattern::AsPattern(_, inner, name) if matches!(**inner, Pattern::WildcardPattern(_)) => {
            let binding = &scope[&name.to_canonical_string()].name;
            quote!(#binding)
        }
        Pattern::UnitPattern(_) => quote!(()),
        Pattern::TuplePattern(_, fields) => {
            let types = match subject {
                Type::Tuple(_, types) => types.as_slice(),
                _ => &[],
            };
            let fields = fields
                .iter()
                .zip(types)
                .map(|(field, tpe)| pattern(field, tpe, scope, renderer, context))
                .collect::<Outcome<Vec<_>>>()?;
            quote!((#(#fields,)*))
        }
        Pattern::LiteralPattern(_, literal) => match literal {
            Literal::Bool(value) => quote!(#value),
            Literal::Char(value) => quote!(#value),
            Literal::Integer(value) => {
                let value = value
                    .to_string()
                    .parse::<i64>()
                    .map_err(|_| error("RS_VALUE", "Integer pattern outside i64 range"))?;
                quote!(#value)
            }
            _ => return Err(error("RS_VALUE", "Unsupported literal pattern")),
        },
        Pattern::ConstructorPattern(_, name, fields) => {
            let family = context
                .constructors(subject)
                .map_err(|e| error("RS_VALUE", e))?
                .ok_or_else(|| error("RS_VALUE", "Unknown constructor family"))?;
            let constructor = family
                .iter()
                .find(|constructor| constructor.name == *name)
                .ok_or_else(|| error("RS_VALUE", "Unknown constructor"))?;
            let mut fields = fields
                .iter()
                .zip(&constructor.arguments)
                .map(|(field, tpe)| pattern(field, tpe, scope, renderer, context))
                .collect::<Outcome<Vec<_>>>()?;
            let path = match name.to_canonical_string().as_str() {
                "morphir/SDK:maybe#just" => quote!(::std::option::Option::Some),
                "morphir/SDK:maybe#nothing" => quote!(::std::option::Option::None),
                "morphir/SDK:result#ok" => quote!(::std::result::Result::Ok),
                "morphir/SDK:result#err" => quote!(::std::result::Result::Err),
                _ => {
                    let Type::Reference(_, subject_name, _) = subject else {
                        return Err(error("RS_VALUE", "Constructor subject must be named"));
                    };
                    let key = subject_name.to_canonical_string();
                    let declaration = renderer
                        .package
                        .declarations
                        .iter()
                        .find(|declaration| declaration.fqname == key)
                        .ok_or_else(|| {
                            error("RS_VALUE", "External constructor patterns are unsupported")
                        })?;
                    let Body::Custom(access, constructors) = &declaration.body else {
                        return Err(error("RS_VALUE", "Constructor requires custom type"));
                    };
                    if *access != Access::Public {
                        return Err(error(
                            "RS_VALUE",
                            "Patterns on private constructor representations are unsupported",
                        ));
                    }
                    let types: Vec<_> = constructors
                        .iter()
                        .flat_map(|constructor| constructor.args.iter().map(|arg| &arg.arg_type))
                        .collect();
                    let original = constructors
                        .iter()
                        .find(|constructor| constructor.name == name.local_name)
                        .expect("validated constructor");
                    for argument in &original.args {
                        validate_pattern_payload(&argument.arg_type, declaration, renderer)?;
                    }
                    if !renderer.markers(declaration, &types, None)?.is_empty() {
                        fields.push(quote!(_));
                    }
                    let path = &renderer.symbols[&key].path;
                    let variant = type_name(&name.local_name)?;
                    quote!(#path::#variant)
                }
            };
            if fields.is_empty() {
                path
            } else {
                quote!(#path(#(#fields),*))
            }
        }
        _ => return Err(error("RS_VALUE", "Unsupported Rust pattern")),
    })
}

fn validate_pattern_payload(
    tpe: &Type,
    owner: &super::ir::Declaration,
    renderer: &Renderer<'_>,
) -> Outcome<()> {
    validate_signature(tpe)?;
    match tpe {
        Type::Reference(_, name, arguments) => {
            if super::types::recursive(
                renderer.package,
                &owner.fqname,
                &name.to_canonical_string(),
                &mut BTreeSet::new(),
            ) {
                return Err(error(
                    "RS_VALUE",
                    "Patterns on recursively boxed constructor payloads are unsupported",
                ));
            }
            for argument in arguments {
                validate_pattern_payload(argument, owner, renderer)?;
            }
        }
        Type::Tuple(_, fields) => {
            for field in fields {
                validate_pattern_payload(field, owner, renderer)?;
            }
        }
        _ => {}
    }
    Ok(())
}

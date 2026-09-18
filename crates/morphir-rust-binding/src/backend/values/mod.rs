mod callables;
mod context;
mod patterns;
mod reuse;
use super::{
    declarations::visibility,
    ir::{Body, Declaration, Function},
    names::*,
    types::Renderer,
};
use crate::{Outcome, error, values};
use morphir_core::{
    ir::v4::{Access, Literal, Pattern, Type, Value, ValueBody},
    naming::{FQName, Name},
};
use patterns::pattern;
use proc_macro2::{Ident, TokenStream};
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
struct Binding {
    name: Ident,
    tpe: Type,
    id: usize,
}
type Scope = BTreeMap<String, Binding>;

pub(super) fn semantic_context(renderer: &Renderer<'_>) -> Outcome<crate::functions::Context> {
    context::context(renderer)
}

pub(super) fn render(
    renderer: &mut Renderer<'_>,
    function: &Function,
    context: &crate::functions::Context,
) -> Outcome<TokenStream> {
    values::validate_function(&function.definition, context)
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
    let mut arguments = vec![];
    for (text, tpe) in &function.definition.input_types {
        let name =
            field_name(&Name::from_canonical_string(text).map_err(|e| error("RS_NAME", e))?)?;
        reserve(&mut seen, &name)?;
        scope.insert(
            text.clone(),
            Binding {
                name: name.clone(),
                tpe: tpe.clone(),
                id: next,
            },
        );
        next += 1;
        let tpe = renderer.signature(tpe, owner)?;
        arguments.push(quote!(#name: #tpe));
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
    let mut expressions = Expressions {
        renderer,
        owner,
        context,
        next,
    };
    let body = expressions.expression(body, &scope, &mut BTreeSet::new(), false)?;
    Ok(quote!(#doc #vis fn #name #generic (#(#arguments),*) -> #output {#body}))
}
fn validate_signature(tpe: &Type) -> Outcome<()> {
    match tpe {
        Type::Record(..) | Type::ExtensibleRecord(..) => Err(error(
            "RS_VALUE",
            "Function signatures require named types; structural records are unsupported",
        )),
        Type::Tuple(_, elements) | Type::Reference(_, _, elements) => {
            for element in elements {
                validate_signature(element)?;
            }
            Ok(())
        }
        Type::Function(_, input, output) => {
            validate_signature(input)?;
            validate_signature(output)
        }
        _ => Ok(()),
    }
}
struct Expressions<'a, 'b> {
    renderer: &'a mut Renderer<'b>,
    owner: &'a Declaration,
    context: &'a crate::functions::Context,
    next: usize,
}
impl Expressions<'_, '_> {
    fn fresh(&mut self, scope: &Scope) -> Outcome<Ident> {
        loop {
            let candidate = ident(&format!("__morphir_binding_{}", self.next))?;
            self.next += 1;
            if !scope.values().any(|binding| binding.name == candidate) {
                return Ok(candidate);
            }
        }
    }
    fn infer(&self, value: &Value, scope: &Scope) -> Outcome<Type> {
        let environment = scope
            .iter()
            .map(|(name, binding)| (name.clone(), binding.tpe.clone()))
            .collect();
        values::infer(value, &environment, self.context).map_err(|e| error("RS_VALUE", e))
    }
    fn expression(
        &mut self,
        value: &Value,
        scope: &Scope,
        moved: &mut BTreeSet<usize>,
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
                let rust_name = &binding.name;
                if reuse::reusable(&binding.tpe) {
                    if borrow {
                        quote!(#rust_name)
                    } else {
                        reuse::value(quote!(#rust_name), &binding.tpe)
                    }
                } else {
                    if !borrow && !crate::functions::copy_type(&binding.tpe) {
                        moved.insert(binding.id);
                    }
                    quote!(#rust_name)
                }
            }
            Value::Tuple(_, fields) => {
                let fields = fields
                    .iter()
                    .map(|f| self.expression(f, scope, moved, false))
                    .collect::<Outcome<Vec<_>>>()?;
                quote!((#(#fields,)*))
            }
            Value::IfThenElse(_, condition, yes, no) => {
                let condition = self.expression(condition, scope, moved, false)?;
                let mut yes_moves = moved.clone();
                let mut no_moves = moved.clone();
                let yes = self.expression(yes, scope, &mut yes_moves, false)?;
                let no = self.expression(no, scope, &mut no_moves, false)?;
                moved.extend(yes_moves);
                moved.extend(no_moves);
                quote!(if #condition {#yes} else {#no})
            }
            Value::PatternMatch(_, subject, cases) => {
                let subject_type = self.infer(subject, scope)?;
                let subject = self.expression(subject, scope, moved, false)?;
                let mut arms = vec![];
                let mut all_moves = moved.clone();
                for case in cases {
                    let bindings =
                        crate::patterns::bindings(&case.0, &subject_type, &self.context.patterns)
                            .map_err(|e| error("RS_VALUE", e))?;
                    let mut arm_scope = scope.clone();
                    for (name, tpe) in bindings {
                        let rust_name = self.fresh(&arm_scope)?;
                        arm_scope.insert(
                            name,
                            Binding {
                                name: rust_name,
                                tpe,
                                id: self.next,
                            },
                        );
                        self.next += 1;
                    }
                    let pattern = pattern(
                        &case.0,
                        &subject_type,
                        &arm_scope,
                        self.renderer,
                        &self.context.patterns,
                    )?;
                    let mut arm_moves = moved.clone();
                    let body = self.expression(&case.1, &arm_scope, &mut arm_moves, false)?;
                    all_moves.extend(arm_moves);
                    arms.push(quote!(#pattern=>{#body}));
                }
                moved.extend(all_moves);
                quote!(match #subject {#(#arms,)*})
            }
            Value::LetDefinition(_, name, definition, continuation) => {
                let ValueBody::Expression(body) = &definition.body else {
                    return Err(error("RS_VALUE", "Local expression required"));
                };
                let body = self.expression(body, scope, moved, false)?;
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
                        tpe: definition
                            .output_type
                            .clone()
                            .expect("validated local type"),
                        id: self.next,
                    },
                );
                self.next += 1;
                let continuation = self.expression(continuation, &scope, moved, false)?;
                quote!({let #rust_name=#body;#continuation})
            }
            Value::Apply(..) => {
                if let Ok((operator, left, right)) = values::comparison(value) {
                    let borrowed = if let Value::Variable(_, name) = left {
                        scope
                            .get(&name.to_canonical_string())
                            .filter(|b| !crate::functions::copy_type(&b.tpe))
                            .map(|b| b.id)
                    } else {
                        None
                    };
                    let left = self.expression(left, scope, moved, true)?;
                    let right = self.expression(right, scope, moved, true)?;
                    if borrowed.is_some_and(|id| moved.contains(&id)) {
                        return Err(error(
                            "RS_VALUE",
                            "Comparison moves its borrowed left operand",
                        ));
                    }
                    let operator: TokenStream = operator.symbol().parse().expect("known operator");
                    quote!((#left) #operator (#right))
                } else {
                    self.application(value, scope, moved)?
                }
            }
            Value::Reference(..) => self.reference(value, scope)?,
            Value::Lambda(..) => self.lambda(value, scope, moved)?,
            _ => return Err(error("RS_VALUE", "Unsupported Rust value expression")),
        })
    }
}

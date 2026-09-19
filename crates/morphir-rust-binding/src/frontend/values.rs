//! The deliberately small, typed expression subset supported by the Rust binding.
mod callable_types;
mod calls;
mod literals;
mod signatures;
use callable_types::CallableShape;
use signatures::Signature;
pub(super) use signatures::{collect, shared};
mod patterns;
mod storage;

use super::types::Context;
use crate::Outcome;
use morphir_core::ir::classic::{module::ModuleValueDefinition, value::ValueArgument, *};
use std::collections::{BTreeMap, BTreeSet};
use syn::{ext::IdentExt, spanned::Spanned};

type Expression = Value<Attrs, Type<Attrs>>;
type Typed = (Expression, Type<Attrs>);
#[derive(Clone)]
struct Binding {
    name: Name,
    ty: Type<Attrs>,
}
#[derive(Clone, Default)]
struct Scope {
    bindings: BTreeMap<String, Binding>,
    moved: BTreeSet<String>,
}
struct Lower<'a, 'b> {
    context: &'a Context<'b>,
    parameters: Vec<syn::Ident>,
    next: usize,
    reserved: Vec<Name>,
    functions: &'a BTreeMap<String, Signature>,
    shapes: BTreeMap<String, CallableShape>,
    lambdas: BTreeMap<String, CallableShape>,
}

pub(super) fn lower(
    context: &Context<'_>,
    function: &syn::ItemFn,
    functions: &BTreeMap<String, Signature>,
    shared: &crate::functions::Context,
) -> Outcome<ModuleValueDefinition<Attrs, Type<Attrs>>> {
    let signature = &function.sig;
    if signature.asyncness.is_some()
        || signature.constness.is_some()
        || !matches!(signature.safety, syn::Safety::Default)
        || signature.abi.is_some()
        || signature.variadic.is_some()
    {
        return Err(context.source.error(
            signature.span(),
            "RS_VALUE_SIGNATURE",
            "Values require safe synchronous non-const free functions without an ABI or variadics",
        ));
    }
    let doc = context.source.attributes(&function.attrs, false)?;
    let mut lower = Lower {
        context,
        parameters: context.source.generics(&signature.generics)?,
        next: 0,
        reserved: vec![],
        functions,
        shapes: BTreeMap::new(),
        lambdas: BTreeMap::new(),
    };
    let mut scope = Scope::default();
    let mut input_types = vec![];
    let mut names = vec![];
    for input in &signature.inputs {
        let syn::FnArg::Typed(argument) = input else {
            return Err(lower.error(input, "Parameters must be named typed arguments"));
        };
        context.source.attributes(&argument.attrs, false)?;
        let ident = lower.pattern(&argument.pat)?;
        names.push(ident.clone());
        let name = context.source.name(ident)?;
        let ty = lower.ty(&argument.ty)?;
        lower.reserved.push(name.clone());
        lower
            .shapes
            .insert(format!("{name:?}"), lower.source_shape(&argument.ty)?);
        scope.bindings.insert(
            ident.unraw().to_string(),
            Binding {
                name: name.clone(),
                ty: ty.clone(),
            },
        );
        input_types.push(ValueArgument {
            name,
            annotation: ty.clone(),
            ty,
        });
    }
    context.source.unique(names)?;
    let output_type = match &signature.output {
        syn::ReturnType::Default => Type::Unit(Attrs::None),
        syn::ReturnType::Type(_, ty) => lower.ty(ty)?,
    };
    for parameter in &lower.parameters {
        let name = context.source.name(parameter)?;
        if !contains_variable(&output_type, &name)
            && !input_types
                .iter()
                .any(|input| contains_variable(&input.ty, &name))
        {
            return Err(lower.error(parameter, "Generic parameters must occur in the function signature; unused parameters cannot be represented in Morphir IR"));
        }
    }
    let (body, actual) = lower.block(&function.block, &mut scope, Some(&output_type))?;
    lower.same(&function.block, &output_type, &actual)?;
    lower.check_shape(
        &function.block,
        &functions[&signature.ident.unraw().to_string()].output_shape,
        &body,
    )?;
    let definition = ValueDefinition {
        input_types,
        output_type,
        body,
    };
    let migrated =
        morphir_core::migration::migrate_value_definition(&definition, &mut Default::default())
            .map_err(|e| lower.error(&function.block, &format!("{e:?}")))?;
    crate::values::validate_function(&migrated, shared)
        .map_err(|e| lower.error(&function.block, &e))?;
    Ok((
        context.source.name(&signature.ident)?,
        AccessControlled {
            access: context.source.visibility(&function.vis)?,
            value: Documented::new(doc, definition),
        },
    ))
}

fn scalar(module: &str, name: &str) -> Type<Attrs> {
    Type::Reference(Attrs::None, sdk(module, name), vec![])
}
fn sdk(module: &str, name: &str) -> FQName {
    FQName::new(
        Path::new(vec!["Morphir".parse().unwrap(), "SDK".parse().unwrap()]),
        Path::new(vec![module.parse().unwrap()]),
        name.parse().unwrap(),
    )
}
fn copy_type(ty: &Type<Attrs>) -> bool {
    matches!(ty, Type::Unit(_))
        || [
            scalar("Basics", "Bool"),
            scalar("Basics", "Int"),
            scalar("Basics", "Float"),
            scalar("Char", "Char"),
        ]
        .contains(ty)
        || matches!(ty, Type::Tuple(_, fields) if fields.iter().all(copy_type))
}

fn contains_variable(ty: &Type<Attrs>, name: &Name) -> bool {
    match ty {
        Type::Variable(_, variable) => variable == name,
        Type::Reference(_, _, arguments) | Type::Tuple(_, arguments) => {
            arguments.iter().any(|ty| contains_variable(ty, name))
        }
        Type::Function(_, input, output) => {
            contains_variable(input, name) || contains_variable(output, name)
        }
        Type::Record(_, fields) => fields
            .iter()
            .any(|field| contains_variable(&field.ty, name)),
        Type::ExtensibleRecord(_, row, fields) => {
            row == name
                || fields
                    .iter()
                    .any(|field| contains_variable(&field.ty, name))
        }
        Type::Unit(_) => false,
    }
}
impl Lower<'_, '_> {
    fn ty(&self, ty: &syn::Type) -> Outcome<Type<Attrs>> {
        self.check_storage_type(ty)?;
        self.executable_type(ty)
    }

    fn error(&self, node: &impl Spanned, message: &str) -> morphir_extension_sdk::Diagnostic {
        self.context
            .source
            .error(node.span(), "RS_VALUE_UNSUPPORTED", message)
    }
    fn same(
        &self,
        node: &impl Spanned,
        expected: &Type<Attrs>,
        actual: &Type<Attrs>,
    ) -> Outcome<()> {
        if expected == actual {
            Ok(())
        } else {
            // Debug names contain process-local interner IDs. The wire form
            // keeps diagnostics stable between native and WASM executions.
            let expected = serde_json::to_string(expected).expect("IR types serialize");
            let actual = serde_json::to_string(actual).expect("IR types serialize");
            Err(self.context.source.error(
                node.span(),
                "RS_VALUE_TYPE",
                format!("Expected {expected}, found {actual}"),
            ))
        }
    }
    fn pattern<'a>(&self, pattern: &'a syn::Pat) -> Outcome<&'a syn::Ident> {
        if let syn::Pat::Ident(p) = pattern
            && p.by_ref.is_none()
            && p.mutability.is_none()
            && p.subpat.is_none()
        {
            self.context.source.attributes(&p.attrs, false)?;
            self.context.source.name(&p.ident)?;
            Ok(&p.ident)
        } else {
            Err(self.error(pattern, "Only immutable named bindings are supported"))
        }
    }
    fn fresh(&mut self) -> Name {
        loop {
            let name: Name = format!("morphirLocal{}", self.next).parse().unwrap();
            self.next += 1;
            if !self.reserved.contains(&name) {
                self.reserved.push(name.clone());
                return name;
            }
        }
    }
    fn block(
        &mut self,
        block: &syn::Block,
        outer: &mut Scope,
        expected: Option<&Type<Attrs>>,
    ) -> Outcome<Typed> {
        let mut scope = outer.clone();
        let result = self.statements(&block.stmts, &mut scope, expected)?;
        outer.moved.extend(scope.moved);
        Ok(result)
    }
    fn statements(
        &mut self,
        statements: &[syn::Stmt],
        scope: &mut Scope,
        expected: Option<&Type<Attrs>>,
    ) -> Outcome<Typed> {
        let Some((first, rest)) = statements.split_first() else {
            return Ok((
                Value::Unit(Type::Unit(Attrs::None)),
                Type::Unit(Attrs::None),
            ));
        };
        match first {
            syn::Stmt::Local(local) => {
                self.context.source.attributes(&local.attrs, false)?;
                let (pattern, annotation) = match &local.pat {
                    syn::Pat::Type(p) => (&*p.pat, Some(&*p.ty)),
                    other => (other, None),
                };
                let ident = self.pattern(pattern)?.clone();
                let Some(init) = &local.init else {
                    return Err(self.error(local, "Let bindings require an initializer"));
                };
                if init.diverge.is_some() {
                    return Err(self.error(local, "Let-else is unsupported"));
                }
                let declared = annotation.map(|a| self.ty(a)).transpose()?;
                let (body, ty) = self.expression_expected(&init.expr, scope, declared.as_ref())?;
                if let Some(annotation) = annotation {
                    self.same(annotation, &self.ty(annotation)?, &ty)?;
                    self.check_shape(annotation, &self.source_shape(annotation)?, &body)?;
                }
                let name = self.fresh();
                let shape = match annotation {
                    Some(annotation) => self.source_shape(annotation)?,
                    None => self.shape(&body)?,
                };
                self.shapes.insert(format!("{name:?}"), shape);
                scope.bindings.insert(
                    ident.unraw().to_string(),
                    Binding {
                        name: name.clone(),
                        ty: ty.clone(),
                    },
                );
                let (continuation, result_type) = self.statements(rest, scope, expected)?;
                Ok((
                    Value::LetDefinition(
                        result_type.clone(),
                        name,
                        Box::new(ValueDefinition {
                            input_types: vec![],
                            output_type: ty,
                            body,
                        }),
                        Box::new(continuation),
                    ),
                    result_type,
                ))
            }
            syn::Stmt::Expr(expression, None) if rest.is_empty() => {
                self.expression_expected(expression, scope, expected)
            }
            _ => Err(self.error(
                first,
                "Only immutable let bindings followed by a tail expression are supported",
            )),
        }
    }
    fn expression(&mut self, expression: &syn::Expr, scope: &mut Scope) -> Outcome<Typed> {
        self.expression_expected(expression, scope, None)
    }
    fn expression_expected(
        &mut self,
        expression: &syn::Expr,
        scope: &mut Scope,
        expected: Option<&Type<Attrs>>,
    ) -> Outcome<Typed> {
        let (mut value, ty) = self.expression_inner(expression, scope, expected)?;

        match &mut value {
            Value::Literal(a, _)
            | Value::Variable(a, _)
            | Value::Tuple(a, _)
            | Value::PatternMatch(a, _, _)
            | Value::IfThenElse(a, _, _, _)
            | Value::LetDefinition(a, _, _, _)
            | Value::Apply(a, _, _)
            | Value::Reference(a, _)
            | Value::Lambda(a, _, _)
            | Value::Unit(a) => *a = ty.clone(),
            _ => unreachable!("supported expression"),
        }
        Ok((value, ty))
    }
    fn expression_inner(
        &mut self,
        expression: &syn::Expr,
        scope: &mut Scope,
        expected: Option<&Type<Attrs>>,
    ) -> Outcome<Typed> {
        match expression {
            syn::Expr::Paren(p) => {
                self.context.source.attributes(&p.attrs, false)?;
                self.expression_expected(&p.expr, scope, expected)
            }
            syn::Expr::Group(g) => {
                self.context.source.attributes(&g.attrs, false)?;
                self.expression_expected(&g.expr, scope, expected)
            }
            syn::Expr::Block(b) if b.label.is_none() => {
                self.context.source.attributes(&b.attrs, false)?;
                self.block(&b.block, scope, expected)
            }
            syn::Expr::Path(p) => self.path(p, scope, expected),
            syn::Expr::Call(call) => self.call(call, scope, expected),
            syn::Expr::Closure(closure) => self.closure(closure, scope),
            syn::Expr::Tuple(t) => {
                self.context.source.attributes(&t.attrs, false)?;
                if t.elems.is_empty() {
                    return Ok((
                        Value::Unit(Type::Unit(Attrs::None)),
                        Type::Unit(Attrs::None),
                    ));
                }
                let elements = t
                    .elems
                    .iter()
                    .enumerate()
                    .map(|(index, e)| {
                        let hint = match expected {
                            Some(Type::Tuple(_, fields)) => fields.get(index),
                            _ => None,
                        };
                        self.expression_expected(e, scope, hint)
                    })
                    .collect::<Outcome<Vec<_>>>()?;
                let (values, types) = elements.into_iter().unzip();
                Ok((
                    Value::Tuple(Type::Unit(Attrs::None), values),
                    Type::Tuple(Attrs::None, types),
                ))
            }
            syn::Expr::Lit(l) => {
                self.context.source.attributes(&l.attrs, false)?;
                self.literal(&l.lit, false)
            }
            syn::Expr::Unary(u) if matches!(u.op, syn::UnOp::Neg(_)) => {
                self.context.source.attributes(&u.attrs, false)?;
                if let syn::Expr::Lit(l) = &*u.expr {
                    self.literal(&l.lit, true)
                } else {
                    Err(self.error(expression, "Negation is supported only on numeric literals"))
                }
            }
            syn::Expr::Unary(u) if matches!(u.op, syn::UnOp::Not(_)) => {
                self.context.source.attributes(&u.attrs, false)?;
                let (condition, ty) = self.expression(&u.expr, scope)?;
                self.same(expression, &scalar("Basics", "Bool"), &ty)?;
                Ok((conditional(condition, boolean(false), boolean(true)), ty))
            }
            syn::Expr::Match(m) => self.match_expression(m, scope, expected),
            syn::Expr::If(i) => {
                self.context.source.attributes(&i.attrs, false)?;
                let (condition, ty) = self.expression(&i.cond, scope)?;
                self.same(&i.cond, &scalar("Basics", "Bool"), &ty)?;
                let mut yes_scope = scope.clone();
                let (yes, yes_type) = self.block(&i.then_branch, &mut yes_scope, expected)?;
                let (no, no_type) = if let Some((_, otherwise)) = &i.else_branch {
                    self.expression_expected(otherwise, scope, expected)?
                } else {
                    (
                        Value::Unit(Type::Unit(Attrs::None)),
                        Type::Unit(Attrs::None),
                    )
                };
                scope.moved.extend(yes_scope.moved);
                self.same(expression, &yes_type, &no_type)?;
                Ok((conditional(condition, yes, no), yes_type))
            }
            syn::Expr::Binary(b) => {
                self.context.source.attributes(&b.attrs, false)?;
                let (left, left_type) = self.expression(&b.left, scope)?;
                let (right, right_type) = self.expression(&b.right, scope)?;
                self.same(expression, &left_type, &right_type)?;
                let boolean_type = scalar("Basics", "Bool");
                let value = match b.op {
                    syn::BinOp::And(_) | syn::BinOp::Or(_) => {
                        self.same(expression, &boolean_type, &left_type)?;
                        if matches!(b.op, syn::BinOp::And(_)) {
                            conditional(left, right, boolean(false))
                        } else {
                            conditional(left, boolean(true), right)
                        }
                    }
                    _ => {
                        let name = match b.op {
                            syn::BinOp::Eq(_) => "equal",
                            syn::BinOp::Ne(_) => "notEqual",
                            syn::BinOp::Lt(_) => "lessThan",
                            syn::BinOp::Le(_) => "lessThanOrEqual",
                            syn::BinOp::Gt(_) => "greaterThan",
                            syn::BinOp::Ge(_) => "greaterThanOrEqual",
                            _ => return Err(self.error(expression, "Unsupported binary operator")),
                        };
                        if ![
                            scalar("Basics", "Bool"),
                            scalar("Basics", "Int"),
                            scalar("Basics", "Float"),
                            scalar("Char", "Char"),
                        ]
                        .contains(&left_type)
                        {
                            return Err(self.error(
                                expression,
                                "Comparisons require supported scalar operands",
                            ));
                        }
                        Value::Apply(
                            boolean_type.clone(),
                            Box::new(Value::Apply(
                                Type::Function(
                                    Attrs::None,
                                    Box::new(right_type.clone()),
                                    Box::new(boolean_type.clone()),
                                ),
                                Box::new(Value::Reference(
                                    Type::Function(
                                        Attrs::None,
                                        Box::new(left_type.clone()),
                                        Box::new(Type::Function(
                                            Attrs::None,
                                            Box::new(right_type.clone()),
                                            Box::new(boolean_type.clone()),
                                        )),
                                    ),
                                    sdk("Basics", name),
                                )),
                                Box::new(left),
                            )),
                            Box::new(right),
                        )
                    }
                };
                Ok((value, boolean_type))
            }
            _ => Err(self.error(expression, "Unsupported Rust expression")),
        }
    }
}
fn boolean(value: bool) -> Expression {
    Value::Literal(scalar("Basics", "Bool"), Literal::Bool(value))
}
fn conditional(condition: Expression, yes: Expression, no: Expression) -> Expression {
    Value::IfThenElse(
        Type::Unit(Attrs::None),
        Box::new(condition),
        Box::new(yes),
        Box::new(no),
    )
}

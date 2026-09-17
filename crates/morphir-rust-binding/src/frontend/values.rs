//! The deliberately small, typed expression subset supported by the Rust binding.
mod literals;

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
}

pub(super) fn lower(
    context: &Context<'_>,
    function: &syn::ItemFn,
) -> Outcome<ModuleValueDefinition<Attrs, Type<Attrs>>> {
    let signature = &function.sig;
    if signature.asyncness.is_some()
        || signature.constness.is_some()
        || signature.unsafety.is_some()
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
    let (body, actual) = lower.block(&function.block, &mut scope)?;
    lower.same(&function.block, &output_type, &actual)?;
    let definition = ValueDefinition {
        input_types,
        output_type,
        body,
    };
    let migrated =
        morphir_core::migration::migrate_value_definition(&definition, &mut Default::default())
            .map_err(|e| lower.error(&function.block, &format!("{e:?}")))?;
    crate::values::validate_function(&migrated).map_err(|e| lower.error(&function.block, &e))?;
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
        self.context.ty(ty, &self.parameters)
    }

    // Type-only lowering erases Box; executable lowering cannot confuse it with
    // a Copy scalar or use a boxed Boolean directly as a condition.
    fn check_storage_type(&self, ty: &syn::Type) -> Outcome<()> {
        match ty {
            syn::Type::Path(path) => {
                for segment in &path.path.segments {
                    if segment.ident.unraw() == "Box"
                        && !self.context.symbols.contains_key("Box")
                        && !self.parameters.iter().any(|p| p.unraw() == "Box")
                    {
                        return Err(self.error(ty, "Box types are not supported in executable function signatures or local annotations"));
                    }
                    if let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments {
                        for argument in &arguments.args {
                            if let syn::GenericArgument::Type(ty) = argument {
                                self.check_storage_type(ty)?;
                            }
                        }
                    }
                }
            }
            syn::Type::Tuple(tuple) => {
                for ty in &tuple.elems {
                    self.check_storage_type(ty)?;
                }
            }
            syn::Type::Paren(paren) => self.check_storage_type(&paren.elem)?,
            syn::Type::Group(group) => self.check_storage_type(&group.elem)?,
            _ => {}
        }
        Ok(())
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
            Err(self.context.source.error(
                node.span(),
                "RS_VALUE_TYPE",
                format!("Expected {expected:?}, found {actual:?}"),
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
    fn block(&mut self, block: &syn::Block, outer: &mut Scope) -> Outcome<Typed> {
        let mut scope = outer.clone();
        let result = self.statements(&block.stmts, &mut scope)?;
        outer.moved.extend(scope.moved);
        Ok(result)
    }
    fn statements(&mut self, statements: &[syn::Stmt], scope: &mut Scope) -> Outcome<Typed> {
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
                let (body, ty) = self.expression(&init.expr, scope)?;
                if let Some(annotation) = annotation {
                    self.same(annotation, &self.ty(annotation)?, &ty)?;
                }
                let name = self.fresh();
                scope.bindings.insert(
                    ident.unraw().to_string(),
                    Binding {
                        name: name.clone(),
                        ty: ty.clone(),
                    },
                );
                let (continuation, result_type) = self.statements(rest, scope)?;
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
                self.expression(expression, scope)
            }
            _ => Err(self.error(
                first,
                "Only immutable let bindings followed by a tail expression are supported",
            )),
        }
    }
    fn expression(&mut self, expression: &syn::Expr, scope: &mut Scope) -> Outcome<Typed> {
        let (mut value, ty) = self.expression_inner(expression, scope)?;
        match &mut value {
            Value::Literal(a, _)
            | Value::Variable(a, _)
            | Value::Tuple(a, _)
            | Value::IfThenElse(a, _, _, _)
            | Value::LetDefinition(a, _, _, _)
            | Value::Apply(a, _, _)
            | Value::Unit(a) => *a = ty.clone(),
            _ => unreachable!("supported expression"),
        }
        Ok((value, ty))
    }
    fn expression_inner(&mut self, expression: &syn::Expr, scope: &mut Scope) -> Outcome<Typed> {
        match expression {
            syn::Expr::Paren(p) => {
                self.context.source.attributes(&p.attrs, false)?;
                self.expression(&p.expr, scope)
            }
            syn::Expr::Group(g) => {
                self.context.source.attributes(&g.attrs, false)?;
                self.expression(&g.expr, scope)
            }
            syn::Expr::Block(b) if b.label.is_none() => {
                self.context.source.attributes(&b.attrs, false)?;
                self.block(&b.block, scope)
            }
            syn::Expr::Path(p)
                if p.qself.is_none()
                    && p.path.leading_colon.is_none()
                    && p.path.segments.len() == 1
                    && matches!(p.path.segments[0].arguments, syn::PathArguments::None) =>
            {
                self.context.source.attributes(&p.attrs, false)?;
                let ident = &p.path.segments[0].ident;
                self.context.source.name(ident)?;
                let Some(binding) = scope.bindings.get(&ident.unraw().to_string()) else {
                    return Err(self.error(expression, "Unbound local or parameter"));
                };
                let key = format!("{:?}", binding.name);
                if !copy_type(&binding.ty) && !scope.moved.insert(key) {
                    return Err(
                        self.error(expression, "Reusing a moved non-Copy value is unsupported")
                    );
                }
                Ok((
                    Value::Variable(Type::Unit(Attrs::None), binding.name.clone()),
                    binding.ty.clone(),
                ))
            }
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
                    .map(|e| self.expression(e, scope))
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
            syn::Expr::If(i) => {
                self.context.source.attributes(&i.attrs, false)?;
                let (condition, ty) = self.expression(&i.cond, scope)?;
                self.same(&i.cond, &scalar("Basics", "Bool"), &ty)?;
                let mut yes_scope = scope.clone();
                let (yes, yes_type) = self.block(&i.then_branch, &mut yes_scope)?;
                let (no, no_type) = if let Some((_, otherwise)) = &i.else_branch {
                    self.expression(otherwise, scope)?
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

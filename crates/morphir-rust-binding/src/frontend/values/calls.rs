//! Saturated Rust calls and explicitly typed closures lower to curried IR nodes.
use super::*;
mod instantiate;
use instantiate::{substitute, unify};

impl Lower<'_, '_> {
    pub(super) fn executable_type(&self, ty: &syn::Type) -> Outcome<Type<Attrs>> {
        fn expand(lower: &Lower<'_, '_>, ty: Type<Attrs>) -> Outcome<Type<Attrs>> {
            Ok(match ty {
                Type::Reference(a, n, args) => {
                    let args = args
                        .into_iter()
                        .map(|a| expand(lower, a))
                        .collect::<Outcome<Vec<_>>>()?;
                    if let Some(alias) = lower.context.items.iter().find_map(|i| match i {
                        syn::Item::Type(t)
                            if FQName::new(
                                lower.context.package.clone(),
                                lower.context.module.clone(),
                                lower.context.source.name(&t.ident).ok()?,
                            ) == n =>
                        {
                            Some(t)
                        }
                        _ => None,
                    }) {
                        let params = lower.context.source.generics(&alias.generics)?;
                        let body = lower.context.ty(&alias.ty, &params)?;
                        let vars = params
                            .iter()
                            .zip(args.clone())
                            .map(|(p, a)| Ok((format!("{:?}", lower.context.source.name(p)?), a)))
                            .collect::<Outcome<_>>()?;
                        let expanded = expand(lower, substitute(&body, &vars))?;
                        fn contains_callable(ty: &Type<Attrs>) -> bool {
                            match ty {
                                Type::Function(..) => true,
                                Type::Tuple(_, xs) | Type::Reference(_, _, xs) => {
                                    xs.iter().any(contains_callable)
                                }
                                _ => false,
                            }
                        }
                        if contains_callable(&expanded) {
                            return Ok(expanded);
                        }
                    }
                    Type::Reference(a, n, args)
                }
                Type::Function(a, i, o) => Type::Function(
                    a,
                    Box::new(expand(lower, *i)?),
                    Box::new(expand(lower, *o)?),
                ),
                Type::Tuple(a, ts) => Type::Tuple(
                    a,
                    ts.into_iter()
                        .map(|t| expand(lower, t))
                        .collect::<Outcome<_>>()?,
                ),
                other => other,
            })
        }
        expand(self, self.context.ty(ty, &self.parameters)?)
    }
    fn named(&self, path: &syn::ExprPath, scope: &Scope) -> Option<Signature> {
        if path.qself.is_some()
            || path.path.leading_colon.is_some()
            || path.path.segments.len() != 1
        {
            return None;
        }
        let name = path.path.segments[0].ident.unraw().to_string();
        if scope.bindings.contains_key(&name) {
            None
        } else {
            self.functions.get(&name).cloned()
        }
    }
    fn substitutions(
        &self,
        path: &syn::ExprPath,
        sig: &Signature,
    ) -> Outcome<BTreeMap<String, Type<Attrs>>> {
        let types = match &path.path.segments[0].arguments {
            syn::PathArguments::None => return Ok(BTreeMap::new()),
            syn::PathArguments::AngleBracketed(args) => args
                .args
                .iter()
                .map(|arg| match arg {
                    syn::GenericArgument::Type(t) => self.ty(t),
                    _ => Err(self.error(arg, "Function generic arguments must be types")),
                })
                .collect::<Outcome<Vec<_>>>()?,
            _ => return Err(self.error(path, "Unsupported function generic arguments")),
        };
        if types.len() != sig.parameters.len() {
            return Err(self.error(path, "Incorrect number of function generic arguments"));
        }
        Ok(sig
            .parameters
            .iter()
            .zip(types)
            .map(|(p, t)| (format!("{p:?}"), t))
            .collect())
    }
    fn reference(
        &self,
        path: &syn::ExprPath,
        sig: &Signature,
        substitutions: &BTreeMap<String, Type<Attrs>>,
    ) -> Outcome<Typed> {
        if sig
            .parameters
            .iter()
            .any(|p| !substitutions.contains_key(&format!("{p:?}")))
        {
            return Err(self.error(
                path,
                "Function generic arguments cannot be inferred; provide explicit type arguments",
            ));
        }
        let ty = substitute(&sig.ty(), substitutions);
        Ok((Value::Reference(ty.clone(), sig.name.clone()), ty))
    }
    pub(super) fn path(
        &mut self,
        path: &syn::ExprPath,
        scope: &mut Scope,
        expected: Option<&Type<Attrs>>,
    ) -> Outcome<Typed> {
        self.context.source.attributes(&path.attrs, false)?;
        if let Some(sig) = self.named(path, scope) {
            let mut substitutions = self.substitutions(path, &sig)?;
            if let Some(expected) = expected.filter(|t| matches!(t, Type::Function(..))) {
                let expected = if sig.inputs.is_empty() {
                    if let Type::Function(_, _, out) = expected {
                        &**out
                    } else {
                        expected
                    }
                } else {
                    expected
                };
                let flexible = sig.parameters.iter().map(|p| format!("{p:?}")).collect();
                unify(&sig.ty(), expected, &flexible, &mut substitutions)
                    .map_err(|e| self.error(path, &e))?;
            }

            let (reference, ty) = self.reference(path, &sig, &substitutions)?;
            if sig.inputs.is_empty() {
                let name = self.fresh();
                let shapes = self.generic_shapes(path, &sig, &substitutions)?;
                self.lambdas
                    .insert(format!("{name:?}"), sig.shape().substitute(&shapes));
                let ty =
                    Type::Function(Attrs::None, Box::new(Type::Unit(Attrs::None)), Box::new(ty));
                return Ok((
                    Value::Lambda(
                        ty.clone(),
                        Pattern::As(
                            Type::Unit(Attrs::None),
                            Box::new(Pattern::Wildcard(Type::Unit(Attrs::None))),
                            name,
                        ),
                        Box::new(reference),
                    ),
                    ty,
                ));
            }
            if !sig.parameters.is_empty() {
                let shapes = self.generic_shapes(path, &sig, &substitutions)?;
                return Ok(self.shaped(reference, ty, sig.shape().substitute(&shapes)));
            }
            return Ok((reference, ty));
        }
        if path.qself.is_some()
            || path.path.leading_colon.is_some()
            || path.path.segments.len() != 1
            || !matches!(path.path.segments[0].arguments, syn::PathArguments::None)
        {
            return Err(self.error(
                path,
                "Only local values and same-module functions are supported",
            ));
        }
        let ident = &path.path.segments[0].ident;
        let Some(binding) = scope.bindings.get(&ident.unraw().to_string()) else {
            return Err(self.error(path, "Unknown local value or non-expression function"));
        };
        let key = format!("{:?}", binding.name);
        if !copy_type(&binding.ty)
            && !self
                .shapes
                .get(&key)
                .is_some_and(|shape| shape.copy(&binding.ty))
            && !scope.moved.insert(key)
        {
            return Err(self.error(path, "Reusing a moved non-Copy value is unsupported"));
        }
        Ok((
            Value::Variable(binding.ty.clone(), binding.name.clone()),
            binding.ty.clone(),
        ))
    }
    pub(super) fn call(
        &mut self,
        call: &syn::ExprCall,
        scope: &mut Scope,
        expected: Option<&Type<Attrs>>,
    ) -> Outcome<Typed> {
        self.context.source.attributes(&call.attrs, false)?;
        if let syn::Expr::Path(path) = &*call.func
            && let Some(sig) = self.named(path, scope)
        {
            if call.args.len() != sig.inputs.len() {
                return Err(self.error(
                    call,
                    "Rust calls must supply exactly the declared number of arguments",
                ));
            }
            let mut variables = self.substitutions(path, &sig)?;
            let flexible = sig.parameters.iter().map(|n| format!("{n:?}")).collect();
            if let Some(expected) = expected {
                unify(&sig.output, expected, &flexible, &mut variables)
                    .map_err(|e| self.error(call, &e))?;
            }
            let mut shape_variables = self.generic_shapes(path, &sig, &BTreeMap::new())?;
            let contextual_types = variables.clone();
            let mut arguments = Vec::new();
            for ((expr, input), shape) in call.args.iter().zip(&sig.inputs).zip(&sig.input_shapes) {
                let hint = substitute(input, &variables);
                // An inferred item/closure identity is not a pointer coercion
                // site. Its erased IR function type would lose that distinction.
                let hint = (!shape.substitute(&shape_variables).has_identity()).then_some(&hint);
                let (argument, actual) = self.expression_expected(expr, scope, hint)?;
                unify(input, &actual, &flexible, &mut variables)
                    .map_err(|e| self.error(expr, &e))?;
                shape
                    .bind(
                        &self.shape(&argument)?,
                        &contextual_types,
                        &mut shape_variables,
                    )
                    .map_err(|e| self.error(expr, &e))?;
                self.check_shape(expr, &shape.substitute(&shape_variables), &argument)?;
                arguments.push(argument);
            }
            let (mut value, mut ty) = self.reference(path, &sig, &variables)?;
            for argument in arguments {
                let Type::Function(_, _, out) = ty else {
                    unreachable!("signature arity")
                };
                ty = *out;
                value = Value::Apply(ty.clone(), Box::new(value), Box::new(argument));
            }
            if !sig.parameters.is_empty() {
                return Ok(self.shaped(value, ty, sig.output_shape.substitute(&shape_variables)));
            }
            return Ok((value, ty));
        }
        let (mut value, mut ty) = self.expression(&call.func, scope)?;
        let CallableShape::Function { inputs, .. } = self.shape(&value)? else {
            return Err(self.error(call, "Call target is not a Rust function or closure"));
        };
        if inputs.len() != call.args.len() {
            return Err(self.error(
                call,
                "Rust calls must supply exactly the declared number of arguments",
            ));
        }
        if inputs.is_empty() {
            let Type::Function(_, input, output) = ty else {
                return Err(self.error(call, "Call target requires a function type"));
            };
            self.same(call, &Type::Unit(Attrs::None), &input)?;
            ty = *output;
            value = Value::Apply(
                ty.clone(),
                Box::new(value),
                Box::new(Value::Unit(Type::Unit(Attrs::None))),
            );
        } else {
            for (expr, shape) in call.args.iter().zip(inputs) {
                let Type::Function(_, input, output) = ty else {
                    return Err(self.error(call, "Call target requires a function type"));
                };
                let (argument, actual) = self.expression_expected(expr, scope, Some(&input))?;
                self.same(expr, &input, &actual)?;
                self.check_shape(expr, &shape, &argument)?;
                ty = *output;
                value = Value::Apply(ty.clone(), Box::new(value), Box::new(argument));
            }
        }
        Ok((value, ty))
    }
    pub(super) fn closure(
        &mut self,
        closure: &syn::ExprClosure,
        outer: &mut Scope,
    ) -> Outcome<Typed> {
        self.context.source.attributes(&closure.attrs, false)?;
        // syn 3 records `static` (coroutine) and `use` closures in `modifiers`.
        if closure.asyncness.is_some()
            || closure.modifiers.require_empty().is_err()
            || closure.constness.is_some()
            || closure.lifetimes.is_some()
        {
            return Err(self.error(closure, "Only synchronous immutable closures are supported"));
        }
        let mut scope = outer.clone();
        let mut inputs = Vec::new();
        let mut shapes = Vec::new();
        let mut patterns = Vec::new();
        let mut bound = BTreeSet::new();
        for parameter in &closure.inputs {
            let syn::Pat::Type(parameter) = parameter else {
                return Err(self.error(
                    parameter,
                    "Lambda parameters require explicit type annotations",
                ));
            };
            let ty = self.ty(&parameter.ty)?;
            let shape = self.source_shape(&parameter.ty)?;
            fn irrefutable_syntax(p: &syn::Pat) -> bool {
                match p {
                    syn::Pat::Ident(_) | syn::Pat::Wild(_) => true,
                    syn::Pat::Tuple(t) => t.elems.iter().all(irrefutable_syntax),
                    syn::Pat::Paren(p) => irrefutable_syntax(&p.pat),
                    _ => false,
                }
            }
            if !irrefutable_syntax(&parameter.pat) {
                return Err(self.error(
                    parameter,
                    "Lambda parameters require irrefutable immutable patterns",
                ));
            }
            let mut pattern = self.match_pattern(&parameter.pat, &ty, &mut scope, &mut bound)?;
            self.lambda_key(&mut pattern);
            self.install_shapes(&pattern, &shape);
            patterns.push(pattern);
            inputs.push(ty);
            shapes.push(shape);
        }
        if patterns.is_empty() {
            let name = self.fresh();
            patterns.push(Pattern::As(
                Type::Unit(Attrs::None),
                Box::new(Pattern::Wildcard(Type::Unit(Attrs::None))),
                name,
            ));
            inputs.push(Type::Unit(Attrs::None));
        }
        let declared = match &closure.output {
            syn::ReturnType::Default => None,
            syn::ReturnType::Type(_, ty) => Some(self.ty(ty)?),
        };
        let (mut body, mut ty) =
            self.expression_expected(&closure.body, &mut scope, declared.as_ref())?;

        if let syn::ReturnType::Type(_, declared) = &closure.output {
            self.same(declared, &self.ty(declared)?, &ty)?;
            self.check_shape(declared, &self.source_shape(declared)?, &body)?;
        }
        let output_shape = match &closure.output {
            syn::ReturnType::Type(_, declared) => self.source_shape(declared)?,
            syn::ReturnType::Default => self.shape(&body)?,
        };
        let first_name =
            callable_types::pattern_key(&patterns[0]).expect("lambda pattern identity");
        for (pattern, input) in patterns.into_iter().zip(inputs).rev() {
            ty = Type::Function(Attrs::None, Box::new(input), Box::new(ty));
            body = Value::Lambda(ty.clone(), pattern, Box::new(body));
        }
        let migrated = morphir_core::migration::migrate_value(&body, &mut Default::default())
            .map_err(|e| self.error(closure, &format!("{e:?}")))?;
        let captures = crate::functions::free_variables(&migrated);
        for capture in &captures {
            let binding = outer
                .bindings
                .values()
                .find(|b| {
                    morphir_core::migration::migrate_name(&b.name, &Default::default())
                        .is_ok_and(|n| n.to_canonical_string() == *capture)
                })
                .ok_or_else(|| self.error(closure, "Unknown lambda capture"))?;
            if !copy_type(&binding.ty) {
                return Err(self.error(
                    closure,
                    "Lambda captures require supported immutable Copy types",
                ));
            }
        }
        self.lambdas.insert(
            first_name,
            CallableShape::Function {
                inputs: shapes,
                output: Box::new(output_shape),
                kind: callable_types::CallableKind::Closure {
                    identity: self.next,
                    captures,
                },
            },
        );
        self.next += 1;
        Ok((body, ty))
    }
}

impl Lower<'_, '_> {
    fn generic_shapes(
        &self,
        path: &syn::ExprPath,
        sig: &Signature,
        types: &BTreeMap<String, Type<Attrs>>,
    ) -> Outcome<BTreeMap<String, CallableShape>> {
        let mut shapes = types
            .iter()
            .map(|(n, t)| (n.clone(), CallableShape::from_ir(t)))
            .collect::<BTreeMap<_, _>>();
        if let syn::PathArguments::AngleBracketed(args) = &path.path.segments[0].arguments {
            for (parameter, arg) in sig.parameters.iter().zip(&args.args) {
                if let syn::GenericArgument::Type(ty) = arg {
                    shapes.insert(format!("{parameter:?}"), self.source_shape(ty)?);
                }
            }
        }
        Ok(shapes)
    }
}

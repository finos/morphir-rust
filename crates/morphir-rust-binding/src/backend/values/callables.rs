//! Lower curried Morphir applications to Rust calls and owned callable values.
use super::*;

impl Expressions<'_, '_> {
    fn named_path(
        &mut self,
        reference: &Value,
        scope: &Scope,
    ) -> Outcome<(TokenStream, Vec<Type>)> {
        let Value::Reference(_, name) = reference else {
            return Err(error("RS_VALUE", "Expected named function reference"));
        };
        let key = name.to_canonical_string();
        let definition = self
            .renderer
            .package
            .functions
            .iter()
            .find(|f| f.owner.fqname == key)
            .ok_or_else(|| error("RS_VALUE", format!("Unknown executable function {name}")))?;
        if definition.owner.access != Access::Public && definition.owner.module != self.owner.module
        {
            return Err(error(
                "RS_VALUE",
                "Calls to private functions in other modules are unsupported",
            ));
        }
        let module = module_names(&definition.owner.module)?;
        let rust_name = field_name(&definition.owner.name)?;
        let signature = self
            .context
            .signatures
            .get(&key)
            .expect("registered function")
            .clone();
        let actual = self.infer(reference, scope)?;
        let substitutions = signature
            .substitutions(&actual)
            .map_err(|e| error("RS_VALUE", e))?;
        let generics = signature
            .parameters
            .iter()
            .map(|parameter| {
                let tpe = substitutions
                    .get(&parameter.to_canonical_string())
                    .ok_or_else(|| error("RS_VALUE", "Unresolved generic function parameter"))?;
                self.renderer.signature(tpe, self.owner)
            })
            .collect::<Outcome<Vec<_>>>()?;
        let path = if generics.is_empty() {
            quote!(crate::#(#module::)*#rust_name)
        } else {
            quote!(crate::#(#module::)*#rust_name::<#(#generics),*>)
        };
        let mut remaining = &actual;
        let mut inputs = vec![];
        for _ in &signature.inputs {
            let Type::Function(_, input, output) = remaining else {
                return Err(error("RS_VALUE", "Function reference arity mismatch"));
            };
            inputs.push((**input).clone());
            remaining = output;
        }
        Ok((path, inputs))
    }
    pub(super) fn reference(&mut self, value: &Value, scope: &Scope) -> Outcome<TokenStream> {
        let (path, inputs) = self.named_path(value, scope)?;
        if inputs.is_empty() {
            return Ok(quote!(#path()));
        }
        // Intermediate curried closures retain the preceding arguments. Their
        // repeated invocation must not consume those retained values.
        if inputs[..inputs.len() - 1]
            .iter()
            .any(|tpe| !crate::functions::copy_type(tpe))
        {
            return Err(error(
                "RS_VALUE",
                "Curried function values require Copy arguments before the final argument",
            ));
        }
        let full_type = self.infer(value, scope)?;
        let mut scope = scope.clone();
        let mut arguments = vec![];
        for tpe in &inputs {
            let argument = self.fresh(&scope)?;
            scope.insert(
                format!("__adapter_{}", self.next),
                Binding {
                    name: argument.clone(),
                    tpe: tpe.clone(),
                    id: self.next,
                },
            );
            self.next += 1;
            arguments.push(argument);
        }
        let mut body = quote!(#path(#(#arguments),*));
        let mut remaining = &full_type;
        let mut types = vec![];
        for _ in &inputs {
            types.push(remaining.clone());
            let Type::Function(_, _, output) = remaining else {
                unreachable!("checked function type")
            };
            remaining = output;
        }
        for ((argument, input), function_type) in arguments.iter().zip(&inputs).zip(types).rev() {
            let Type::Function(_, _, output) = &function_type else {
                unreachable!()
            };
            let input = self.renderer.signature(input, self.owner)?;
            let output = self.renderer.signature(output, self.owner)?;
            let function_type = self.renderer.signature(&function_type, self.owner)?;
            let binding = self.fresh(&scope)?;
            body = quote!({let #binding: #function_type = ::std::rc::Rc::new(move |#argument: #input| -> #output {#body});#binding});
        }
        Ok(body)
    }
    pub(super) fn application(
        &mut self,
        value: &Value,
        scope: &Scope,
        moved: &mut BTreeSet<usize>,
    ) -> Outcome<TokenStream> {
        let mut root = value;
        let mut arguments = vec![];
        while let Value::Apply(_, function, argument) = root {
            arguments.push(&**argument);
            root = function;
        }
        arguments.reverse();
        let direct_arity = if let Value::Reference(_, name) = root {
            self.context
                .signatures
                .get(&name.to_canonical_string())
                .map(|signature| signature.inputs.len())
        } else {
            None
        };
        let (mut expression, remaining) = if let Some(arity) =
            direct_arity.filter(|arity| *arity > 0 && arguments.len() >= *arity)
        {
            let (path, _) = self.named_path(root, scope)?;
            let actuals = arguments[..arity]
                .iter()
                .map(|argument| self.expression(argument, scope, moved, false))
                .collect::<Outcome<Vec<_>>>()?;
            (quote!(#path(#(#actuals),*)), &arguments[arity..])
        } else {
            (
                self.expression(root, scope, moved, true)?,
                arguments.as_slice(),
            )
        };
        for argument in remaining {
            let argument = self.expression(argument, scope, moved, false)?;
            expression = quote!((#expression)(#argument));
        }
        Ok(expression)
    }
    pub(super) fn lambda(
        &mut self,
        value: &Value,
        scope: &Scope,
        moved: &BTreeSet<usize>,
    ) -> Outcome<TokenStream> {
        let Value::Lambda(_, parameter, body) = value else {
            unreachable!()
        };
        let full_type = self.infer(value, scope)?;
        let Type::Function(_, input, output) = &full_type else {
            return Err(error("RS_VALUE", "Lambda requires a function type"));
        };
        for free in crate::functions::free_variables(value) {
            let binding = scope
                .get(&free)
                .ok_or_else(|| error("RS_VALUE", format!("Unknown capture {free}")))?;
            if moved.contains(&binding.id) {
                return Err(error(
                    "RS_VALUE",
                    format!("Capture {free} is used after it was moved"),
                ));
            }
            if !crate::functions::copy_type(&binding.tpe) {
                return Err(error(
                    "RS_VALUE",
                    "Only immutable Copy captures are supported",
                ));
            }
        }
        let bindings = crate::patterns::bindings(parameter, input, &self.context.patterns)
            .map_err(|e| error("RS_VALUE", e))?;
        let mut lambda_scope = scope.clone();
        for (name, tpe) in bindings {
            let rust_name = self.fresh(&lambda_scope)?;
            lambda_scope.insert(
                name,
                Binding {
                    name: rust_name,
                    tpe,
                    id: self.next,
                },
            );
            self.next += 1;
        }
        let parameter = pattern(
            parameter,
            input,
            &lambda_scope,
            self.renderer,
            &self.context.patterns,
        )?;
        let body = self.expression(body, &lambda_scope, &mut moved.clone(), false)?;
        let input = self.renderer.signature(input, self.owner)?;
        let output = self.renderer.signature(output, self.owner)?;
        let full_type = self.renderer.signature(&full_type, self.owner)?;
        let binding = self.fresh(&lambda_scope)?;
        Ok(
            quote!({let #binding: #full_type = ::std::rc::Rc::new(move |#parameter: #input| -> #output {#body});#binding}),
        )
    }
}

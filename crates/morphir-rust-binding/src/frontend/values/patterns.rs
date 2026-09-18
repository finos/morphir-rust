//! Typed source patterns and arm-local binding scopes.
use super::*;

type TypedPattern = Pattern<Type<Attrs>>;
struct Constructor {
    name: FQName,
    fields: Vec<(Option<String>, Type<Attrs>)>,
    shape: Shape,
}
#[derive(PartialEq)]
enum Shape {
    Unit,
    Tuple,
    Named,
}

impl Lower<'_, '_> {
    pub(super) fn match_expression(
        &mut self,
        expression: &syn::ExprMatch,
        scope: &mut Scope,
        expected: Option<&Type<Attrs>>,
    ) -> Outcome<Typed> {
        self.context.source.attributes(&expression.attrs, false)?;
        let (subject, subject_type) = self.expression(&expression.expr, scope)?;
        let mut cases = Vec::new();
        let mut result_type = None;
        let mut moved = scope.moved.clone();
        for arm in &expression.arms {
            self.context.source.attributes(&arm.attrs, false)?;
            if arm.guard.is_some() {
                return Err(self.error(arm, "Match guards are unsupported"));
            }
            let mut arm_scope = scope.clone();
            let pattern = self.match_pattern(
                &arm.pat,
                &subject_type,
                &mut arm_scope,
                &mut BTreeSet::new(),
            )?;
            self.install_shapes(&pattern, &self.shape(&subject)?);
            let (body, ty) = self.expression_expected(&arm.body, &mut arm_scope, expected)?;
            if let Some(expected) = &result_type {
                self.same(&arm.body, expected, &ty)?;
            } else {
                result_type = Some(ty);
            }
            moved.extend(arm_scope.moved);
            cases.push((pattern, body));
        }
        let Some(ty) = result_type else {
            return Err(self.error(expression, "Empty matches are unsupported"));
        };
        // Shared validation checks constructor membership, bindings and exhaustiveness
        // after the completed function has migrated to v4.
        scope.moved = moved;
        Ok((
            Value::PatternMatch(ty.clone(), Box::new(subject), cases),
            ty,
        ))
    }

    pub(super) fn match_pattern(
        &mut self,
        pattern: &syn::Pat,
        ty: &Type<Attrs>,
        scope: &mut Scope,
        bound: &mut BTreeSet<String>,
    ) -> Outcome<TypedPattern> {
        match pattern {
            syn::Pat::Wild(p) => {
                self.context.source.attributes(&p.attrs, false)?;
                Ok(Pattern::Wildcard(ty.clone()))
            }
            syn::Pat::Paren(p) => {
                self.context.source.attributes(&p.attrs, false)?;
                self.match_pattern(&p.pat, ty, scope, bound)
            }
            syn::Pat::Ident(p)
                if matches!(
                    p.ident.unraw().to_string().as_str(),
                    "Some" | "None" | "Ok" | "Err"
                ) && !self.local_value_name(&p.ident.unraw().to_string()) =>
            {
                self.context.source.attributes(&p.attrs, false)?;
                if p.by_ref.is_some() || p.mutability.is_some() || p.subpat.is_some() {
                    return Err(self.error(p, "Constructor patterns cannot use binding modifiers"));
                }
                let ident = &p.ident;
                let path: syn::Path = syn::parse_quote!(#ident);
                let constructor = self.constructor(&path, ty)?;
                if constructor.shape != Shape::Unit {
                    return Err(self.error(p, "Constructor pattern requires its fields"));
                }
                Ok(Pattern::Constructor(ty.clone(), constructor.name, vec![]))
            }
            syn::Pat::Ident(p) => {
                let ident = self.pattern(pattern)?.clone();
                let source_name = ident.unraw().to_string();
                self.require_variant_qualification(&ident, ty)?;
                if self.context.items.iter().any(|item| matches!(item, syn::Item::Struct(s) if s.ident.unraw() == source_name && !matches!(s.fields, syn::Fields::Named(_)))) {
                    return Err(self.error(p, "Struct constructor patterns are unsupported"));
                }
                if !bound.insert(source_name.clone()) {
                    return Err(self.error(p, "Duplicate pattern binding"));
                }
                let name = self.fresh();
                scope.bindings.insert(
                    source_name,
                    Binding {
                        name: name.clone(),
                        ty: ty.clone(),
                    },
                );
                Ok(Pattern::As(
                    ty.clone(),
                    Box::new(Pattern::Wildcard(ty.clone())),
                    name,
                ))
            }
            syn::Pat::Tuple(p) => {
                self.context.source.attributes(&p.attrs, false)?;
                if p.elems.is_empty() {
                    self.same(p, &Type::Unit(Attrs::None), ty)?;
                    return Ok(Pattern::Unit(ty.clone()));
                }
                let Type::Tuple(_, fields) = ty else {
                    return Err(self.error(p, "Tuple pattern requires a tuple subject"));
                };
                if p.elems.len() != fields.len() {
                    return Err(self.error(p, "Tuple pattern arity mismatch"));
                }
                Ok(Pattern::Tuple(
                    ty.clone(),
                    p.elems
                        .iter()
                        .zip(fields)
                        .map(|(p, t)| self.match_pattern(p, t, scope, bound))
                        .collect::<Outcome<_>>()?,
                ))
            }
            syn::Pat::Lit(p) => {
                self.context.source.attributes(&p.attrs, false)?;
                if !matches!(
                    p.lit,
                    syn::Lit::Bool(_) | syn::Lit::Int(_) | syn::Lit::Char(_)
                ) {
                    return Err(self.error(
                        p,
                        "Only Boolean, i64 and character literal patterns are supported",
                    ));
                }
                let (value, actual) = self.literal(&p.lit, false)?;
                self.literal_pattern(p, value, &actual, ty)
            }
            syn::Pat::Const(p) => Err(self.error(p, "Constant patterns are unsupported")),
            syn::Pat::Path(p) if p.qself.is_none() => {
                self.context.source.attributes(&p.attrs, false)?;
                let constructor = self.constructor(&p.path, ty)?;
                if constructor.shape != Shape::Unit {
                    return Err(self.error(p, "Constructor pattern requires its fields"));
                }
                Ok(Pattern::Constructor(ty.clone(), constructor.name, vec![]))
            }
            syn::Pat::TupleStruct(p) if p.qself.is_none() => {
                self.context.source.attributes(&p.attrs, false)?;
                let constructor = self.constructor(&p.path, ty)?;
                if constructor.shape != Shape::Tuple || constructor.fields.len() != p.elems.len() {
                    return Err(self.error(p, "Constructor pattern field shape or arity mismatch"));
                }
                let arguments = p
                    .elems
                    .iter()
                    .zip(&constructor.fields)
                    .map(|(p, (_, t))| self.match_pattern(p, t, scope, bound))
                    .collect::<Outcome<_>>()?;
                Ok(Pattern::Constructor(
                    ty.clone(),
                    constructor.name,
                    arguments,
                ))
            }
            syn::Pat::Struct(p) if p.qself.is_none() => {
                self.context.source.attributes(&p.attrs, false)?;
                let constructor = self.constructor(&p.path, ty)?;
                if constructor.shape != Shape::Named {
                    return Err(self.error(p, "Named patterns require a named-field enum variant"));
                }
                let mut supplied = BTreeMap::new();
                for field in &p.fields {
                    self.context.source.attributes(&field.attrs, false)?;
                    let syn::Member::Named(name) = &field.member else {
                        return Err(self.error(field, "Expected a named enum field"));
                    };
                    let key = name.unraw().to_string();
                    if !constructor
                        .fields
                        .iter()
                        .any(|(name, _)| name.as_ref() == Some(&key))
                        || supplied.insert(key, &*field.pat).is_some()
                    {
                        return Err(self.error(field, "Unknown or duplicate enum pattern field"));
                    }
                }
                if p.rest.is_none() && supplied.len() != constructor.fields.len() {
                    return Err(self.error(p, "Enum pattern must include every field or use .."));
                }
                let arguments = constructor
                    .fields
                    .iter()
                    .map(|(name, ty)| match supplied.get(name.as_ref().unwrap()) {
                        Some(pattern) => self.match_pattern(pattern, ty, scope, bound),
                        None => Ok(Pattern::Wildcard(ty.clone())),
                    })
                    .collect::<Outcome<_>>()?;
                Ok(Pattern::Constructor(
                    ty.clone(),
                    constructor.name,
                    arguments,
                ))
            }
            _ => Err(self.error(pattern, "Unsupported Rust match pattern")),
        }
    }

    fn literal_pattern(
        &self,
        node: &impl Spanned,
        value: Expression,
        actual: &Type<Attrs>,
        expected: &Type<Attrs>,
    ) -> Outcome<TypedPattern> {
        self.same(node, expected, actual)?;
        if ![
            scalar("Basics", "Bool"),
            scalar("Basics", "Int"),
            scalar("Char", "Char"),
        ]
        .contains(actual)
        {
            return Err(self.error(node, "Unsupported literal pattern type"));
        }
        let Value::Literal(_, literal) = value else {
            unreachable!("literal lowering")
        };
        Ok(Pattern::Literal(expected.clone(), literal))
    }

    // Resolve aliases only for the Rust variant-name rule. The original subject
    // type and binding annotations remain nominal throughout actual lowering.
    fn resolve_subject_aliases(
        &self,
        subject: &Type<Attrs>,
        active: &mut BTreeSet<String>,
    ) -> Outcome<Type<Attrs>> {
        let Type::Reference(_, name, arguments) = subject else {
            return Ok(subject.clone());
        };
        if &name.package_path != self.context.package || &name.module_path != self.context.module {
            return Ok(subject.clone());
        }
        for item in self.context.items {
            if let syn::Item::Type(alias) = item
                && self.context.source.name(&alias.ident)? == name.local_name
            {
                let parameters = self.context.source.generics(&alias.generics)?;
                if parameters.len() != arguments.len() {
                    return Err(self.error(alias, "Type alias argument arity mismatch"));
                }
                // Expand arguments before entering the alias body, so finite
                // Identity<Identity<E>> nesting is not mistaken for an alias cycle.
                let arguments = arguments
                    .iter()
                    .map(|ty| self.resolve_subject_aliases(ty, active))
                    .collect::<Outcome<Vec<_>>>()?;
                let key = alias.ident.unraw().to_string();
                if !active.insert(key.clone()) {
                    return Err(self.error(alias, "Recursive type aliases are unsupported"));
                }
                let substitutions = parameters
                    .iter()
                    .zip(arguments)
                    .map(|(name, ty)| Ok((self.context.source.name(name)?, ty)))
                    .collect::<Outcome<Vec<_>>>()?;
                let definition = self.context.ty(&alias.ty, &parameters)?;
                let result =
                    self.resolve_subject_aliases(&substitute(&definition, &substitutions), active);
                active.remove(&key);
                return result;
            }
        }
        Ok(subject.clone())
    }

    fn require_variant_qualification(
        &self,
        ident: &syn::Ident,
        subject: &Type<Attrs>,
    ) -> Outcome<()> {
        let resolved = self.resolve_subject_aliases(subject, &mut BTreeSet::new())?;
        let Type::Reference(_, name, _) = &resolved else {
            return Ok(());
        };
        if &name.package_path != self.context.package || &name.module_path != self.context.module {
            return Ok(());
        }
        for item in self.context.items {
            if let syn::Item::Enum(enumeration) = item
                && self.context.source.name(&enumeration.ident)? == name.local_name
                && enumeration
                    .variants
                    .iter()
                    .any(|variant| variant.ident.unraw() == ident.unraw())
            {
                return Err(self.error(
                    ident,
                    "Enum variant patterns require qualification as Enum::Variant",
                ));
            }
        }
        Ok(())
    }

    fn local_value_name(&self, name: &str) -> bool {
        self.context.items.iter().any(|item| match item {
            syn::Item::Fn(f) => f.sig.ident.unraw() == name,
            syn::Item::Struct(s) => s.ident.unraw() == name,
            _ => false,
        })
    }

    fn constructor(&self, path: &syn::Path, subject: &Type<Attrs>) -> Outcome<Constructor> {
        if path.leading_colon.is_some()
            || path
                .segments
                .iter()
                .any(|s| !matches!(s.arguments, syn::PathArguments::None))
        {
            return Err(self.error(
                path,
                "Qualified or generic constructor paths are unsupported",
            ));
        }
        let segments: Vec<_> = path
            .segments
            .iter()
            .map(|s| s.ident.unraw().to_string())
            .collect();
        let (owner, variant) = match segments.as_slice() {
            [owner, variant] => (owner.as_str(), variant.as_str()),
            [variant] if !self.local_value_name(variant) => (
                match variant.as_str() {
                    "Some" | "None" => "Option",
                    "Ok" | "Err" => "Result",
                    _ => {
                        return Err(self.error(
                            path,
                            "Unqualified constructors are supported only for Option and Result",
                        ));
                    }
                },
                variant.as_str(),
            ),
            _ => {
                return Err(self.error(
                    path,
                    "Expected Enum::Variant, Option::Some/None or Result::Ok/Err",
                ));
            }
        };
        let Type::Reference(_, subject_name, arguments) = subject else {
            return Err(self.error(path, "Constructor pattern requires a named subject type"));
        };
        if let Some(enumeration) = self.context.items.iter().find_map(|item| match item {
            syn::Item::Enum(e) if e.ident.unraw() == owner => Some(e),
            _ => None,
        }) {
            let expected = FQName::new(
                self.context.package.clone(),
                self.context.module.clone(),
                self.context.source.name(&enumeration.ident)?,
            );
            if subject_name != &expected {
                return Err(self.error(path, "Constructor does not belong to the subject type"));
            }
            let Some(variant) = enumeration
                .variants
                .iter()
                .find(|v| v.ident.unraw() == variant)
            else {
                return Err(self.error(path, "Unknown enum variant"));
            };
            let parameters = self.context.source.generics(&enumeration.generics)?;
            if parameters.len() != arguments.len() {
                return Err(self.error(path, "Enum type argument arity mismatch"));
            }
            let substitutions = parameters
                .iter()
                .zip(arguments)
                .map(|(p, t)| Ok((self.context.source.name(p)?, t.clone())))
                .collect::<Outcome<Vec<_>>>()?;
            let fields = variant
                .fields
                .iter()
                .map(|f| {
                    self.check_storage_type_with(&f.ty, &parameters)?;
                    Ok((
                        f.ident.as_ref().map(|i| i.unraw().to_string()),
                        substitute(&self.context.ty(&f.ty, &parameters)?, &substitutions),
                    ))
                })
                .collect::<Outcome<_>>()?;
            return Ok(Constructor {
                name: FQName::new(
                    self.context.package.clone(),
                    self.context.module.clone(),
                    self.context.source.name(&variant.ident)?,
                ),
                fields,
                shape: match variant.fields {
                    syn::Fields::Unit => Shape::Unit,
                    syn::Fields::Unnamed(_) => Shape::Tuple,
                    syn::Fields::Named(_) => Shape::Named,
                },
            });
        }
        if self.context.symbols.contains_key(owner)
            || self.parameters.iter().any(|p| p.unraw() == owner)
        {
            return Err(self.error(path, "Constructor requires an enum declaration"));
        }
        let (expected, constructor, payload) = match (owner, variant) {
            ("Option", "Some") if arguments.len() == 1 => (
                sdk("Maybe", "Maybe"),
                sdk("Maybe", "Just"),
                Some(arguments[0].clone()),
            ),
            ("Option", "None") if arguments.len() == 1 => {
                (sdk("Maybe", "Maybe"), sdk("Maybe", "Nothing"), None)
            }
            ("Result", "Ok") if arguments.len() == 2 => (
                sdk("Result", "Result"),
                sdk("Result", "Ok"),
                Some(arguments[1].clone()),
            ),
            ("Result", "Err") if arguments.len() == 2 => (
                sdk("Result", "Result"),
                sdk("Result", "Err"),
                Some(arguments[0].clone()),
            ),
            _ => return Err(self.error(path, "Unknown constructor or type argument arity")),
        };
        if subject_name != &expected {
            return Err(self.error(path, "Constructor does not belong to the subject type"));
        }
        Ok(Constructor {
            name: constructor,
            shape: if payload.is_some() {
                Shape::Tuple
            } else {
                Shape::Unit
            },
            fields: payload.into_iter().map(|ty| (None, ty)).collect(),
        })
    }
}

fn substitute(ty: &Type<Attrs>, parameters: &[(Name, Type<Attrs>)]) -> Type<Attrs> {
    match ty {
        Type::Variable(_, name) => parameters
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.clone())
            .unwrap_or_else(|| ty.clone()),
        Type::Reference(a, n, args) => Type::Reference(
            a.clone(),
            n.clone(),
            args.iter().map(|t| substitute(t, parameters)).collect(),
        ),
        Type::Tuple(a, args) => Type::Tuple(
            a.clone(),
            args.iter().map(|t| substitute(t, parameters)).collect(),
        ),
        Type::Function(a, x, y) => Type::Function(
            a.clone(),
            Box::new(substitute(x, parameters)),
            Box::new(substitute(y, parameters)),
        ),
        Type::Record(a, fields) => Type::Record(
            a.clone(),
            fields
                .iter()
                .map(|f| Field {
                    name: f.name.clone(),
                    ty: substitute(&f.ty, parameters),
                })
                .collect(),
        ),
        Type::ExtensibleRecord(a, n, fields) => Type::ExtensibleRecord(
            a.clone(),
            n.clone(),
            fields
                .iter()
                .map(|f| Field {
                    name: f.name.clone(),
                    ty: substitute(&f.ty, parameters),
                })
                .collect(),
        ),
        Type::Unit(_) => ty.clone(),
    }
}

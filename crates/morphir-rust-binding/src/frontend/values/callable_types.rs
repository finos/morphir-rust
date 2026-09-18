//! Source callable shape: Rust arity and closure identity are distinct from curried IR types.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum CallableShape {
    Plain,
    Variable(String),
    Tuple(Vec<CallableShape>),
    Function {
        inputs: Vec<CallableShape>,
        output: Box<CallableShape>,
        kind: CallableKind,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub(super) enum CallableKind {
    Pointer,
    Item(FQName),
    Closure {
        identity: usize,
        captures: BTreeSet<String>,
    },
}
impl CallableKind {
    fn can_coerce_to_pointer(&self) -> bool {
        match self {
            Self::Pointer | Self::Item(_) => true,
            Self::Closure { captures, .. } => captures.is_empty(),
        }
    }
}
impl CallableShape {
    pub(super) fn pointer(inputs: Vec<Self>, output: Self) -> Self {
        Self::Function {
            inputs,
            output: Box::new(output),
            kind: CallableKind::Pointer,
        }
    }
    pub(super) fn accepts(&self, actual: &Self) -> bool {
        match (self, actual) {
            (Self::Plain, Self::Plain) => true,
            (Self::Variable(a), Self::Variable(b)) => a == b,
            (Self::Tuple(a), Self::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.accepts(b))
            }
            (
                Self::Function {
                    inputs: a,
                    output: ao,
                    kind: ac,
                    ..
                },
                Self::Function {
                    inputs: b,
                    output: bo,
                    kind: bc,
                    ..
                },
            ) => {
                a.len() == b.len()
                    && a.iter().zip(b).all(|(a, b)| a.accepts(b))
                    && ao.accepts(bo)
                    && (ac == bc
                        || matches!(ac, CallableKind::Pointer) && bc.can_coerce_to_pointer())
            }
            _ => false,
        }
    }
    pub(super) fn copy(&self, ty: &Type<Attrs>) -> bool {
        copy_type(ty)
            || matches!(self, Self::Function { .. })
            || match (self, ty) {
                (Self::Tuple(shapes), Type::Tuple(_, types)) => {
                    shapes.len() == types.len()
                        && shapes.iter().zip(types).all(|(shape, ty)| shape.copy(ty))
                }
                _ => false,
            }
    }
}
impl Lower<'_, '_> {
    pub(super) fn source_shape(&self, ty: &syn::Type) -> Outcome<CallableShape> {
        let parameters = self
            .parameters
            .iter()
            .map(|p| {
                Ok((
                    p.unraw().to_string(),
                    CallableShape::Variable(format!("{:?}", self.context.source.name(p)?)),
                ))
            })
            .collect::<Outcome<_>>()?;
        self.source_shape_with(ty, &parameters)
    }
    fn source_shape_with(
        &self,
        ty: &syn::Type,
        parameters: &BTreeMap<String, CallableShape>,
    ) -> Outcome<CallableShape> {
        match ty {
            syn::Type::BareFn(f) => Ok(CallableShape::pointer(
                f.inputs
                    .iter()
                    .map(|i| self.source_shape_with(&i.ty, parameters))
                    .collect::<Outcome<_>>()?,
                match &f.output {
                    syn::ReturnType::Default => CallableShape::Plain,
                    syn::ReturnType::Type(_, t) => self.source_shape_with(t, parameters)?,
                },
            )),
            syn::Type::Tuple(t) if !t.elems.is_empty() => Ok(CallableShape::Tuple(
                t.elems
                    .iter()
                    .map(|t| self.source_shape_with(t, parameters))
                    .collect::<Outcome<_>>()?,
            )),
            syn::Type::Paren(p) => self.source_shape_with(&p.elem, parameters),
            syn::Type::Group(p) => self.source_shape_with(&p.elem, parameters),
            syn::Type::Path(p) if p.qself.is_none() && p.path.segments.len() == 1 => {
                let segment = &p.path.segments[0];
                let name = segment.ident.unraw().to_string();
                if let Some(shape) = parameters.get(&name) {
                    return Ok(shape.clone());
                }
                if let Some(alias) = self.context.items.iter().find_map(|i| match i {
                    syn::Item::Type(a) if a.ident.unraw() == name => Some(a),
                    _ => None,
                }) {
                    let args = match &segment.arguments {
                        syn::PathArguments::AngleBracketed(args) => args
                            .args
                            .iter()
                            .map(|a| match a {
                                syn::GenericArgument::Type(t) => {
                                    self.source_shape_with(t, parameters)
                                }
                                _ => Err(self.error(a, "Only type arguments are supported")),
                            })
                            .collect::<Outcome<Vec<_>>>()?,
                        _ => vec![],
                    };
                    let names = self.context.source.generics(&alias.generics)?;
                    let parameters = names
                        .iter()
                        .zip(args)
                        .map(|(n, a)| (n.unraw().to_string(), a))
                        .collect();
                    return self.source_shape_with(&alias.ty, &parameters);
                }
                Ok(CallableShape::Plain)
            }
            _ => Ok(CallableShape::Plain),
        }
    }
    pub(super) fn shape(&self, value: &Expression) -> Outcome<CallableShape> {
        let plain = CallableShape::Plain;
        Ok(match value {
            Value::Variable(ty, n) => self
                .shapes
                .get(&format!("{n:?}"))
                .cloned()
                .unwrap_or_else(|| CallableShape::from_ir(ty)),
            Value::Reference(_, n) => self
                .functions
                .values()
                .find(|f| f.name == *n)
                .map(|f| {
                    if f.inputs.is_empty() {
                        f.output_shape.clone()
                    } else {
                        f.shape()
                    }
                })
                .unwrap_or(plain),
            Value::Tuple(_, xs) => {
                CallableShape::Tuple(xs.iter().map(|v| self.shape(v)).collect::<Outcome<_>>()?)
            }
            Value::Lambda(_, pattern, _) => self
                .lambdas
                .get(&pattern_key(pattern).unwrap_or_default())
                .cloned()
                .unwrap_or(plain),
            Value::Apply(_, f, _) => match self.shape(f)? {
                CallableShape::Function {
                    mut inputs,
                    output,
                    kind,
                } if inputs.len() > 1 => {
                    inputs.remove(0);
                    CallableShape::Function {
                        inputs,
                        output,
                        kind,
                    }
                }
                CallableShape::Function { output, .. } => *output,
                _ => plain,
            },
            Value::LetDefinition(_, _, _, body) => self.shape(body)?,
            Value::IfThenElse(_, _, a, b) => self.join_shapes(&self.shape(a)?, &self.shape(b)?)?,
            Value::PatternMatch(_, _, cases) => {
                let mut shape = None;
                for (_, v) in cases {
                    let next = self.shape(v)?;
                    shape = Some(match shape {
                        None => next,
                        Some(previous) => self.join_shapes(&previous, &next)?,
                    });
                }
                shape.unwrap_or(plain)
            }
            _ => plain,
        })
    }
    fn join_shapes(&self, a: &CallableShape, b: &CallableShape) -> Outcome<CallableShape> {
        if a == b {
            return Ok(a.clone());
        }
        match (a, b) {
            (
                CallableShape::Function {
                    inputs,
                    output,
                    kind,
                    ..
                },
                CallableShape::Function { kind: other, .. },
            ) if kind.can_coerce_to_pointer() && other.can_coerce_to_pointer() => {
                let pointer = CallableShape::pointer(inputs.clone(), *output.clone());
                if pointer.accepts(b) {
                    return Ok(pointer);
                }
            }
            (CallableShape::Tuple(a), CallableShape::Tuple(b)) if a.len() == b.len() => {
                return Ok(CallableShape::Tuple(
                    a.iter()
                        .zip(b)
                        .map(|(a, b)| self.join_shapes(a, b))
                        .collect::<Outcome<_>>()?,
                ));
            }
            _ => {}
        }
        Err(crate::error(
            "RS_VALUE_TYPE",
            "Branches have incompatible Rust callable types",
        ))
    }
    pub(super) fn check_shape(
        &self,
        node: &impl Spanned,
        expected: &CallableShape,
        actual: &Expression,
    ) -> Outcome<()> {
        if expected.accepts(&self.shape(actual)?) {
            Ok(())
        } else {
            Err(self.error(node,"Rust callable arity or closure capture is incompatible with the expected function pointer type"))
        }
    }
}

pub(super) fn pattern_key(pattern: &Pattern<Type<Attrs>>) -> Option<String> {
    match pattern {
        Pattern::As(_, _, n) => Some(format!("{n:?}")),
        Pattern::Tuple(_, parts) => parts.iter().find_map(pattern_key),
        _ => None,
    }
}
impl Lower<'_, '_> {
    pub(super) fn install_shapes(&mut self, pattern: &Pattern<Type<Attrs>>, shape: &CallableShape) {
        match (pattern, shape) {
            (Pattern::As(_, inner, name), shape) => {
                self.shapes.insert(format!("{name:?}"), shape.clone());
                self.install_shapes(inner, shape);
            }
            (Pattern::Tuple(_, parts), CallableShape::Tuple(shapes)) => {
                for (p, s) in parts.iter().zip(shapes) {
                    self.install_shapes(p, s);
                }
            }
            _ => {}
        }
    }
    pub(super) fn lambda_key(&mut self, pattern: &mut Pattern<Type<Attrs>>) -> String {
        if let Some(key) = pattern_key(pattern) {
            return key;
        }
        if let Pattern::Tuple(_, parts) = pattern
            && let Some(first) = parts.first_mut()
        {
            return self.lambda_key(first);
        }
        let ty = match pattern {
            Pattern::Wildcard(t) | Pattern::Unit(t) | Pattern::Tuple(t, _) => t.clone(),
            _ => unreachable!("lambda patterns are irrefutable"),
        };
        let name = self.fresh();
        let key = format!("{name:?}");
        *pattern = Pattern::As(ty.clone(), Box::new(Pattern::Wildcard(ty)), name);
        key
    }
}
impl CallableShape {
    pub(super) fn substitute(&self, variables: &BTreeMap<String, Self>) -> Self {
        match self {
            Self::Variable(n) => variables.get(n).cloned().unwrap_or_else(|| self.clone()),
            Self::Tuple(xs) => Self::Tuple(xs.iter().map(|x| x.substitute(variables)).collect()),
            Self::Function {
                inputs,
                output,
                kind,
            } => Self::Function {
                inputs: inputs.iter().map(|x| x.substitute(variables)).collect(),
                output: Box::new(output.substitute(variables)),
                kind: kind.clone(),
            },
            _ => self.clone(),
        }
    }
    pub(super) fn bind(
        &self,
        actual: &Self,
        contextual_types: &BTreeMap<String, Type<Attrs>>,
        variables: &mut BTreeMap<String, Self>,
    ) -> Result<(), String> {
        match (self, actual) {
            (Self::Variable(n), actual) => {
                if let Some(previous) = variables.get(n) {
                    if !previous.accepts(actual) {
                        return Err("Inconsistent generic Rust callable types".into());
                    }
                } else {
                    let shape = contextual_types
                        .get(n)
                        .map(|ty| actual.in_context(ty))
                        .unwrap_or_else(|| actual.clone());
                    variables.insert(n.clone(), shape);
                }
            }
            (Self::Tuple(a), Self::Tuple(b)) => {
                for (a, b) in a.iter().zip(b) {
                    a.bind(b, contextual_types, variables)?;
                }
            }
            (
                Self::Function {
                    inputs: a,
                    output: ao,
                    ..
                },
                Self::Function {
                    inputs: b,
                    output: bo,
                    ..
                },
            ) => {
                for (a, b) in a.iter().zip(b) {
                    a.bind(b, contextual_types, variables)?;
                }
                ao.bind(bo, contextual_types, variables)?;
            }
            _ => {}
        }
        Ok(())
    }
    fn in_context(&self, ty: &Type<Attrs>) -> Self {
        match (self, ty) {
            // The IR type establishes pointer coercion, but cannot recover
            // source arity. Keep that arity from the argument's source shape.
            (Self::Function { inputs, output, .. }, Type::Function(..)) => {
                Self::pointer(inputs.clone(), *output.clone())
            }
            (Self::Tuple(shapes), Type::Tuple(_, types)) => Self::Tuple(
                shapes
                    .iter()
                    .zip(types)
                    .map(|(shape, ty)| shape.in_context(ty))
                    .collect(),
            ),
            _ => self.clone(),
        }
    }
    pub(super) fn from_ir(ty: &Type<Attrs>) -> Self {
        match ty {
            Type::Variable(_, n) => Self::Variable(format!("{n:?}")),
            Type::Tuple(_, xs) if !xs.is_empty() => {
                Self::Tuple(xs.iter().map(Self::from_ir).collect())
            }
            Type::Function(_, i, o) => Self::pointer(vec![Self::from_ir(i)], Self::from_ir(o)),
            _ => Self::Plain,
        }
    }
}
impl Lower<'_, '_> {
    // A hygienic local retains instantiated source callable shape without extending IR attributes.
    pub(super) fn shaped(
        &mut self,
        value: Expression,
        ty: Type<Attrs>,
        shape: CallableShape,
    ) -> Typed {
        let name = self.fresh();
        self.shapes.insert(format!("{name:?}"), shape);
        (
            Value::LetDefinition(
                ty.clone(),
                name.clone(),
                Box::new(ValueDefinition {
                    input_types: vec![],
                    output_type: ty.clone(),
                    body: value,
                }),
                Box::new(Value::Variable(ty.clone(), name)),
            ),
            ty,
        )
    }
}

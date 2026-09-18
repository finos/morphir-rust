use super::*;
use morphir_core::migration::*;

#[derive(Clone)]
pub(in crate::frontend) struct Signature {
    pub name: FQName,
    pub parameters: Vec<Name>,
    pub inputs: Vec<Type<Attrs>>,
    pub output: Type<Attrs>,
    pub(super) input_shapes: Vec<CallableShape>,
    pub(super) output_shape: CallableShape,
}
impl Signature {
    pub(super) fn shape(&self) -> CallableShape {
        CallableShape::Function {
            inputs: self.input_shapes.clone(),
            output: Box::new(self.output_shape.clone()),
            kind: callable_types::CallableKind::Item(self.name.clone()),
        }
    }
    pub(super) fn ty(&self) -> Type<Attrs> {
        function_type(&self.inputs, self.output.clone())
    }
}
pub(super) fn function_type(inputs: &[Type<Attrs>], output: Type<Attrs>) -> Type<Attrs> {
    inputs.iter().rev().fold(output, |out, input| {
        Type::Function(Attrs::None, Box::new(input.clone()), Box::new(out))
    })
}
pub(in crate::frontend) fn collect(context: &Context<'_>) -> Outcome<BTreeMap<String, Signature>> {
    let empty = BTreeMap::new();
    let mut result = BTreeMap::new();
    for item in context.items {
        let syn::Item::Fn(f) = item else {
            continue;
        };
        if super::super::bindings::lower(context, f)?.is_some() {
            continue;
        }
        let lower = Lower {
            context,
            parameters: context.source.generics(&f.sig.generics)?,
            functions: &empty,
            next: 0,
            reserved: vec![],
            shapes: BTreeMap::new(),
            lambdas: BTreeMap::new(),
        };
        let mut inputs = Vec::new();
        let mut input_shapes = Vec::new();
        for arg in &f.sig.inputs {
            let syn::FnArg::Typed(arg) = arg else {
                return Err(lower.error(arg, "Only free function inputs are supported"));
            };
            inputs.push(lower.ty(&arg.ty)?);
            input_shapes.push(lower.source_shape(&arg.ty)?);
        }
        let (output, output_shape) = match &f.sig.output {
            syn::ReturnType::Default => (Type::Unit(Attrs::None), CallableShape::Plain),
            syn::ReturnType::Type(_, t) => (lower.ty(t)?, lower.source_shape(t)?),
        };
        result.insert(
            f.sig.ident.unraw().to_string(),
            Signature {
                name: FQName::new(
                    context.package.clone(),
                    context.module.clone(),
                    context.source.name(&f.sig.ident)?,
                ),
                parameters: lower
                    .parameters
                    .iter()
                    .map(|p| context.source.name(p))
                    .collect::<Outcome<_>>()?,
                inputs,
                output,
                input_shapes,
                output_shape,
            },
        );
    }
    Ok(result)
}
pub(in crate::frontend) fn shared(
    signatures: &BTreeMap<String, Signature>,
    patterns: crate::patterns::Context,
) -> Outcome<crate::functions::Context> {
    let mut result = crate::functions::Context {
        patterns,
        ..Default::default()
    };
    for sig in signatures.values() {
        let mut ctx = MigrationContext::default();
        let mut convert = || -> Result<_, MigrationDiagnostic> {
            Ok((
                migrate_fqname(&sig.name, &ctx.cursor)?.to_canonical_string(),
                crate::functions::Signature {
                    parameters: sig
                        .parameters
                        .iter()
                        .map(|n| migrate_name(n, &ctx.cursor))
                        .collect::<Result<_, _>>()?,
                    inputs: sig
                        .inputs
                        .iter()
                        .map(|t| migrate_type(t, &mut ctx))
                        .collect::<Result<_, _>>()?,
                    output: migrate_type(&sig.output, &mut ctx)?,
                },
            ))
        };
        let (name, sig) = convert().map_err(|e| crate::error("RS_MIGRATION", format!("{e:?}")))?;
        result.signatures.insert(name, sig);
    }
    Ok(result)
}

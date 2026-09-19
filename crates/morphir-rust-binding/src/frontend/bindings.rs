//! Lower annotated free-function signatures; their source bodies are intentionally omitted.
use super::{binding_attributes, types::Context};
use crate::Outcome;
use morphir_core::{
    ir::{classic, v4},
    migration::{MigrationContext, migrate_access, migrate_name, migrate_type},
    traversal::IrCursor,
};
use syn::spanned::Spanned;

pub(super) type Definition = (
    String,
    v4::AccessControlled<v4::module::Documented<v4::ValueDefinition>>,
);

pub(super) fn lower(context: &Context<'_>, function: &syn::ItemFn) -> Outcome<Option<Definition>> {
    let metadata = binding_attributes::parse(context.source, &function.attrs)?;
    let Some(binding) = metadata.binding else {
        return Ok(None);
    };
    let signature = &function.sig;
    if signature.asyncness.is_some()
        || signature.constness.is_some()
        || !matches!(signature.safety, syn::Safety::Default)
        || signature.abi.is_some()
        || signature.variadic.is_some()
    {
        return Err(context.source.error(signature.span(), "RS_BINDING_SIGNATURE", "Bindings require synchronous, safe, non-const free functions without an ABI or variadics"));
    }
    let parameters = context.source.generics(&signature.generics)?;
    let mut inputs = Vec::new();
    for input in &signature.inputs {
        let syn::FnArg::Typed(argument) = input else {
            return Err(context.source.error(
                input.span(),
                "RS_BINDING_SIGNATURE",
                "Binding parameters must be plain named arguments",
            ));
        };
        context.source.attributes(&argument.attrs, false)?;
        let syn::Pat::Ident(pattern) = &*argument.pat else {
            return Err(context.source.error(
                argument.pat.span(),
                "RS_BINDING_SIGNATURE",
                "Binding parameters must be plain named arguments",
            ));
        };
        if pattern.by_ref.is_some() || pattern.mutability.is_some() || pattern.subpat.is_some() {
            return Err(context.source.error(
                pattern.span(),
                "RS_BINDING_SIGNATURE",
                "Binding parameters cannot contain ref, mut or subpatterns",
            ));
        }
        context.source.attributes(&pattern.attrs, false)?;
        inputs.push((
            pattern.ident.clone(),
            lower_type(context, &argument.ty, &parameters)?,
        ));
    }
    context
        .source
        .unique(inputs.iter().map(|(ident, _)| ident.clone()))?;
    let input_types = inputs
        .into_iter()
        .map(|(ident, ty)| Ok((name(context, &ident)?, ty)))
        .collect::<Outcome<_>>()?;
    let output = match &signature.output {
        syn::ReturnType::Type(_, ty) => lower_type(context, ty, &parameters)?,
        syn::ReturnType::Default => migrate_type(
            &classic::Type::Unit(classic::Attrs::None),
            &mut MigrationContext::default(),
        )
        .expect("unit migration is infallible"),
    };
    let visibility = context.source.visibility(&function.vis)?;
    Ok(Some((
        name(context, &signature.ident)?,
        v4::AccessControlled {
            access: migrate_access(&visibility),
            value: v4::module::Documented::new(
                (!metadata.documentation.is_empty()).then(|| metadata.documentation.into()),
                v4::ValueDefinition {
                    input_types,
                    output_type: Some(output),
                    body: binding.into_body(),
                },
            ),
        },
    )))
}

fn name(context: &Context<'_>, ident: &syn::Ident) -> Outcome<String> {
    migrate_name(&context.source.name(ident)?, &IrCursor::default())
        .map(|name| name.to_canonical_string())
        .map_err(|error| {
            context
                .source
                .error(ident.span(), "RS_MIGRATION", format!("{error:?}"))
        })
}

fn lower_type(
    context: &Context<'_>,
    ty: &syn::Type,
    parameters: &[syn::Ident],
) -> Outcome<v4::Type> {
    let classic = context.ty(ty, parameters)?;
    migrate_type(&classic, &mut MigrationContext::default()).map_err(|error| {
        context
            .source
            .error(ty.span(), "RS_MIGRATION", format!("{error:?}"))
    })
}

use super::*;

pub(super) fn pattern(
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
    owner: &super::super::ir::Declaration,
    renderer: &Renderer<'_>,
) -> Outcome<()> {
    validate_signature(tpe)?;
    match tpe {
        Type::Reference(_, name, arguments) => {
            if super::super::types::recursive(
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

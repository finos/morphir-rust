use super::*;

pub(super) fn context(renderer: &Renderer<'_>) -> Outcome<crate::functions::Context> {
    let definitions = renderer
        .package
        .functions
        .iter()
        .map(|function| (function.owner.fqname.clone(), function.definition.clone()))
        .collect();
    crate::functions::check_cycles(&definitions).map_err(|e| error("RS_VALUE", e))?;
    let signatures = renderer
        .package
        .functions
        .iter()
        .map(|function| {
            Ok((
                function.owner.fqname.clone(),
                crate::functions::Signature {
                    parameters: function.owner.params.clone(),
                    inputs: function.definition.input_types.values().cloned().collect(),
                    output: function
                        .definition
                        .output_type
                        .clone()
                        .ok_or_else(|| error("RS_VALUE", "Function output type is required"))?,
                },
            ))
        })
        .collect::<Outcome<_>>()?;
    Ok(crate::functions::Context {
        patterns: pattern_context(renderer)?,
        signatures,
    })
}

fn pattern_context(renderer: &Renderer<'_>) -> Outcome<crate::patterns::Context> {
    let mut context = crate::patterns::Context::default();
    for declaration in &renderer.package.declarations {
        if let Body::Custom(_, constructors) = &declaration.body {
            let prefix = declaration
                .fqname
                .rsplit_once('#')
                .expect("declaration FQName")
                .0;
            context.types.insert(
                declaration.fqname.clone(),
                crate::patterns::CustomType {
                    parameters: declaration.params.clone(),
                    constructors: constructors
                        .iter()
                        .map(|constructor| {
                            Ok(crate::patterns::Constructor {
                                name: FQName::from_canonical_string(&format!(
                                    "{prefix}#{}",
                                    constructor.name.to_canonical_string()
                                ))
                                .map_err(|e| error("RS_NAME", e))?,
                                arguments: constructor
                                    .args
                                    .iter()
                                    .map(|arg| arg.arg_type.clone())
                                    .collect(),
                            })
                        })
                        .collect::<Outcome<_>>()?,
                },
            );
        }
    }
    Ok(context)
}

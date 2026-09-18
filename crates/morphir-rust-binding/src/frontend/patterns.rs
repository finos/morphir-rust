//! Register lowered constructor families for shared pattern validation.
use super::types::Context;
use crate::Outcome;
use morphir_core::{
    ir::classic::{module::ModuleTypeDefinition, *},
    migration::*,
};

pub(super) fn context(
    context: &Context<'_>,
    definitions: &[ModuleTypeDefinition<Attrs>],
) -> Outcome<crate::patterns::Context> {
    let mut result = crate::patterns::Context::default();
    let mut migration = MigrationContext::default();
    for (name, definition) in definitions {
        if let TypeDefinition::Custom(parameters, constructors) = &definition.value.value {
            let fq = |name: &Name| {
                FQName::new(
                    context.package.clone(),
                    context.module.clone(),
                    name.clone(),
                )
            };
            let mut convert = || -> Result<_, morphir_core::migration::MigrationDiagnostic> {
                let name = migrate_fqname(&fq(name), &migration.cursor)?;
                let parameters = parameters
                    .iter()
                    .map(|n| migrate_name(n, &migration.cursor))
                    .collect::<Result<_, _>>()?;
                let constructors = constructors
                    .value
                    .iter()
                    .map(|constructor| {
                        Ok(crate::patterns::Constructor {
                            name: migrate_fqname(&fq(&constructor.name), &migration.cursor)?,
                            arguments: constructor
                                .args
                                .iter()
                                .map(|(_, ty)| migrate_type(ty, &mut migration))
                                .collect::<Result<_, _>>()?,
                        })
                    })
                    .collect::<Result<_, morphir_core::migration::MigrationDiagnostic>>()?;
                Ok((
                    name.to_string(),
                    crate::patterns::CustomType {
                        parameters,
                        constructors,
                    },
                ))
            };
            let (name, custom) =
                convert().map_err(|e| crate::error("RS_MIGRATION", format!("{e:?}")))?;
            result.types.insert(name, custom);
        }
    }
    Ok(result)
}

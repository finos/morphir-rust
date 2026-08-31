use super::*;

mod canonical;
mod mappings;
mod naming;
mod references;
mod tuples;

pub(super) use canonical::canonical_type;
pub(super) use mappings::{source_properties, validate_physical_mappings};

impl Projector<'_> {
    pub(super) fn project_type(
        &mut self,
        schema_namespace: &str,
        owner_source: &str,
        tpe: &TypeExpr,
    ) -> Result<AvroType, AvroDiagnostic> {
        match tpe {
            TypeExpr::Unit => Ok(AvroType::Null),
            TypeExpr::Tuple(elements) => {
                self.project_tuple(schema_namespace, owner_source, elements)
            }
            TypeExpr::Record(_) => Err(AvroDiagnostic::unsupported_morphir_type(
                "anonymous record outside a named alias",
            )),
            TypeExpr::Reference {
                source_name,
                arguments,
            } => self.project_reference(schema_namespace, owner_source, source_name, arguments),
            TypeExpr::Variable(parameter) => Err(AvroDiagnostic::unbound_type_parameter(format!(
                "{parameter} at {owner_source}"
            ))),
            TypeExpr::ExtensibleRecord { .. } | TypeExpr::Function { .. } => {
                Err(AvroDiagnostic::unsupported_morphir_type(format!(
                    "{owner_source}: {}",
                    canonical_type(tpe)
                )))
            }
        }
    }
}

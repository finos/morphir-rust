use super::super::encoding::type_application_digest;
use super::super::*;
use super::canonical_type;

impl Projector<'_> {
    pub(in crate::avro::project) fn invariant_failure(
        &mut self,
        message: impl Into<String>,
    ) -> AvroDiagnostic {
        let error = AvroInternalError::invariant(message);
        if self.internal_failure.is_none() {
            self.internal_failure = Some(error);
        }
        AvroDiagnostic::unsupported_morphir_type("internal projection invariant")
    }

    pub(super) fn specialized_name(
        &mut self,
        info: &DeclarationInfo,
        arguments: &[TypeExpr],
    ) -> Result<AvroFullName, AvroDiagnostic> {
        let readable_name = format!(
            "{}{}",
            info.full_name.name(),
            arguments
                .iter()
                .map(|argument| self.type_suffix(argument))
                .collect::<String>()
        );
        let name = format!(
            "{readable_name}_{}",
            type_application_digest(info.declaration.source_name(), arguments)
        );
        let full_name = AvroFullName::new(info.full_name.namespace().to_owned(), name)?;
        self.registry.claim(
            &full_name.to_string(),
            &format!(
                "{}<{}>",
                info.declaration.source_name(),
                arguments
                    .iter()
                    .map(canonical_type)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        )?;
        Ok(full_name)
    }

    pub(in crate::avro::project) fn type_suffix(&self, tpe: &TypeExpr) -> String {
        match tpe {
            TypeExpr::Reference {
                source_name,
                arguments,
            } => {
                let base = match source_name.as_str() {
                    SDK_BOOL => "Bool".to_owned(),
                    SDK_INT => "Int".to_owned(),
                    SDK_FLOAT => "Float".to_owned(),
                    SDK_STRING => "String".to_owned(),
                    SDK_CHAR => "Char".to_owned(),
                    _ => self
                        .declarations
                        .get(source_name)
                        .map(|info| info.full_name.name().to_owned())
                        .or_else(|| full_name_from_source(source_name).map(|(_, name)| name))
                        .unwrap_or_else(|| upper_camel(source_name)),
                };
                format!(
                    "{base}{}",
                    arguments
                        .iter()
                        .map(|argument| self.type_suffix(argument))
                        .collect::<String>()
                )
            }
            TypeExpr::Variable(name) => upper_camel(name),
            other => {
                let digest = Sha256::digest(canonical_type(other));
                format!(
                    "Type{}",
                    digest[..4]
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>()
                )
            }
        }
    }
}

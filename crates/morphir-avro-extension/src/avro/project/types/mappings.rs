use super::super::*;

impl Projector<'_> {
    pub(super) fn logical_type(
        &self,
        physical: AvroType,
        name: &str,
        properties: Properties,
    ) -> AvroType {
        if self.options.logical_types {
            AvroType::Logical {
                physical: Box::new(physical),
                name: name.to_owned(),
                properties,
            }
        } else {
            physical
        }
    }

    pub(in crate::avro::project) fn mapped_type(
        &self,
        source_name: &str,
        mapping: &crate::TypeMapping,
    ) -> Result<AvroType, AvroDiagnostic> {
        let physical = physical_type(source_name, &mapping.physical_type)?;
        let mut properties = source_properties(source_name);
        if let Some(logical_type) = &mapping.logical_type {
            if logical_type == "decimal" {
                properties.insert(
                    "precision".to_owned(),
                    json!(mapping.precision.unwrap_or(self.options.decimal_precision)),
                );
                properties.insert(
                    "scale".to_owned(),
                    json!(mapping.scale.unwrap_or(self.options.decimal_scale)),
                );
            }
            Ok(AvroType::Logical {
                physical: Box::new(physical),
                name: logical_type.clone(),
                properties,
            })
        } else {
            Ok(AvroType::Annotated {
                physical: Box::new(physical),
                properties,
            })
        }
    }
}

pub(in crate::avro::project) fn validate_physical_mappings(
    options: &AvroOptions,
) -> Result<(), AvroDiagnostic> {
    options
        .type_mappings
        .iter()
        .try_for_each(|(source_name, mapping)| {
            physical_type(source_name, &mapping.physical_type)
                .map(|_| ())
                .map_err(|error| error.with_source(source_name))
        })
}

fn physical_type(source_name: &str, physical_type: &str) -> Result<AvroType, AvroDiagnostic> {
    match physical_type {
        "null" => Ok(AvroType::Null),
        "boolean" => Ok(AvroType::Boolean),
        "int" => Ok(AvroType::Int),
        "long" => Ok(AvroType::Long),
        "float" => Ok(AvroType::Float),
        "double" => Ok(AvroType::Double),
        "bytes" => Ok(AvroType::Bytes),
        "string" => Ok(AvroType::String),
        unsupported => Err(AvroDiagnostic::invalid_option(format!(
            "type_mappings.{source_name}.type has unsupported Avro physical type {unsupported:?}"
        ))),
    }
}

pub(in crate::avro::project) fn source_properties(source_name: &str) -> Properties {
    BTreeMap::from([(
        "morphir.fqname".to_owned(),
        Value::String(source_name.to_owned()),
    )])
}

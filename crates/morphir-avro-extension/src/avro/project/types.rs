use super::*;
use super::{
    declarations::substitute,
    encoding::{canonical_tuple_identity, type_application_digest},
    protocol::{declaration_doc, declaration_type_params},
};

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

    pub(super) fn project_reference(
        &mut self,
        schema_namespace: &str,
        owner_source: &str,
        source_name: &str,
        arguments: &[TypeExpr],
    ) -> Result<AvroType, AvroDiagnostic> {
        if self.invalid_declarations.contains(source_name) {
            return Err(AvroDiagnostic::name_collision(format!(
                "reference to conflicting Morphir declaration {source_name}"
            )));
        }
        if let Some(mapping) = self.options.type_mappings.get(source_name) {
            return self.mapped_type(source_name, mapping);
        }
        match source_name {
            SDK_BOOL if arguments.is_empty() => Ok(AvroType::Boolean),
            SDK_INT if arguments.is_empty() => Ok(AvroType::Long),
            SDK_FLOAT if arguments.is_empty() => Ok(AvroType::Double),
            SDK_STRING if arguments.is_empty() => Ok(AvroType::String),
            SDK_CHAR if arguments.is_empty() => Ok(AvroType::Annotated {
                physical: Box::new(AvroType::String),
                properties: BTreeMap::from([("morphir.type".to_owned(), json!("Char"))]),
            }),
            SDK_MAYBE if arguments.len() == 1 => {
                let value = self.project_type(schema_namespace, owner_source, &arguments[0])?;
                AvroUnion::new(vec![AvroType::Null, value])
                    .map(AvroType::Union)
                    .map_err(AvroDiagnostic::unsupported_morphir_type)
            }
            SDK_LIST if arguments.len() == 1 => Ok(AvroType::Array(
                Box::new(self.project_type(schema_namespace, owner_source, &arguments[0])?),
                Properties::new(),
            )),
            SDK_SET if arguments.len() == 1 => Ok(AvroType::Array(
                Box::new(self.project_type(schema_namespace, owner_source, &arguments[0])?),
                BTreeMap::from([("morphir.collection-kind".to_owned(), json!("set"))]),
            )),
            SDK_DICT if arguments.len() == 2 => {
                if !self.dict_key_is_string_compatible(&arguments[0]) {
                    return Err(AvroDiagnostic::unsupported_morphir_type(format!(
                        "Dict key {}",
                        canonical_type(&arguments[0])
                    )));
                }
                Ok(AvroType::Map(
                    Box::new(self.project_type(schema_namespace, owner_source, &arguments[1])?),
                    Properties::new(),
                ))
            }
            SDK_RESULT if arguments.len() == 2 => {
                self.project_result(schema_namespace, owner_source, &arguments[0], &arguments[1])
            }
            SDK_LOCAL_DATE if arguments.is_empty() => {
                Ok(self.logical_type(AvroType::Int, "date", Properties::new()))
            }
            SDK_LOCAL_TIME if arguments.is_empty() => {
                Ok(self.logical_type(AvroType::Long, "time-micros", Properties::new()))
            }
            SDK_INSTANT | SDK_DATE_TIME if arguments.is_empty() => {
                Ok(self.logical_type(AvroType::Long, "timestamp-micros", Properties::new()))
            }
            SDK_UUID if arguments.is_empty() => {
                Ok(self.logical_type(AvroType::String, "uuid", Properties::new()))
            }
            SDK_DECIMAL if arguments.is_empty() => Ok(self.logical_type(
                AvroType::Bytes,
                "decimal",
                BTreeMap::from([
                    (
                        "precision".to_owned(),
                        json!(self.options.decimal_precision),
                    ),
                    ("scale".to_owned(), json!(self.options.decimal_scale)),
                ]),
            )),
            _ if source_name.starts_with("morphir/SDK:") => {
                Err(AvroDiagnostic::unsupported_morphir_type(source_name))
            }
            _ => self.project_declared_reference(
                schema_namespace,
                owner_source,
                source_name,
                arguments,
            ),
        }
    }

    fn dict_key_is_string_compatible(&self, tpe: &TypeExpr) -> bool {
        self.dict_key_is_string_compatible_inner(tpe, &mut BTreeSet::new())
    }

    fn dict_key_is_string_compatible_inner(
        &self,
        tpe: &TypeExpr,
        resolving: &mut BTreeSet<String>,
    ) -> bool {
        let TypeExpr::Reference {
            source_name,
            arguments,
        } = tpe
        else {
            return false;
        };
        if let Some(mapping) = self.options.type_mappings.get(source_name) {
            return mapping.physical_type == "string";
        }
        if source_name == SDK_STRING && arguments.is_empty() {
            return true;
        }
        if self.options.aliases != Aliases::Inline {
            return false;
        }
        let identity = canonical_type(tpe);
        if !resolving.insert(identity.clone()) {
            return false;
        }
        let declaration = self
            .declarations
            .get(source_name)
            .map(|info| info.declaration.clone());
        let compatible = match declaration {
            Some(TypeDeclaration::Alias {
                type_params, value, ..
            }) if type_params.len() == arguments.len() && !matches!(value, TypeExpr::Record(_)) => {
                let substitutions = type_params
                    .into_iter()
                    .zip(arguments.iter().cloned())
                    .collect::<BTreeMap<_, _>>();
                self.dict_key_is_string_compatible_inner(
                    &substitute(&value, &substitutions),
                    resolving,
                )
            }
            _ => false,
        };
        resolving.remove(&identity);
        compatible
    }

    pub(super) fn project_declared_reference(
        &mut self,
        schema_namespace: &str,
        _owner_source: &str,
        source_name: &str,
        arguments: &[TypeExpr],
    ) -> Result<AvroType, AvroDiagnostic> {
        let Some(info) = self.declarations.get(source_name).cloned() else {
            return Err(if self.options.dependencies == Dependencies::Linked {
                AvroDiagnostic::missing_linked_dependency(source_name)
            } else {
                AvroDiagnostic::unsupported_morphir_type(source_name)
            });
        };
        let type_params = declaration_type_params(&info.declaration);
        if type_params.len() != arguments.len() {
            return Err(AvroDiagnostic::unsupported_morphir_type(source_name));
        }
        let substitutions = type_params
            .iter()
            .cloned()
            .zip(arguments.iter().cloned())
            .collect::<BTreeMap<_, _>>();
        let full_name = if arguments.is_empty() {
            info.full_name.clone()
        } else {
            self.specialized_name(&info, arguments)?
        };
        let alias_value = match &info.declaration {
            TypeDeclaration::Alias { value, .. } => Some(substitute(value, &substitutions)),
            _ => None,
        };
        let is_named = matches!(alias_value, Some(TypeExpr::Record(_)))
            || (alias_value.is_some() && self.options.aliases == Aliases::WrapperRecord)
            || matches!(info.declaration, TypeDeclaration::Custom { .. });
        let ownership = self.declaration_ownership(&info);
        let specialization = arguments
            .iter()
            .map(canonical_type)
            .collect::<Vec<_>>()
            .join(",");
        if let Some(active) = self.active_declarations.get(source_name) {
            if is_named && active == &specialization {
                return Ok(AvroType::Named(full_name));
            }
            return Err(AvroDiagnostic::unsafe_recursion(source_name));
        }
        self.active_declarations
            .insert(source_name.to_owned(), specialization);
        let result = match (&info.declaration, alias_value) {
            (TypeDeclaration::Alias { .. }, Some(value)) if is_named => self
                .project_alias_schema(
                    source_name,
                    &full_name,
                    &value,
                    declaration_doc(&info.declaration),
                    ownership,
                )
                .map(|()| AvroType::Named(full_name)),
            (TypeDeclaration::Alias { .. }, Some(value)) => {
                self.project_type(schema_namespace, source_name, &value)
            }
            (TypeDeclaration::Custom { constructors, .. }, _) => self
                .project_custom(
                    source_name,
                    &full_name,
                    constructors,
                    &substitutions,
                    declaration_doc(&info.declaration),
                    ownership,
                )
                .map(|()| AvroType::Named(full_name)),
            (TypeDeclaration::Opaque { .. }, _) | (TypeDeclaration::Incomplete { .. }, _) => {
                Err(AvroDiagnostic::unsupported_morphir_type(source_name))
            }
            (TypeDeclaration::Alias { .. }, None) => Err(self.invariant_failure(format!(
                "alias declaration {source_name} lost its substituted value"
            ))),
        };
        self.active_declarations.remove(source_name);
        result
    }

    pub(super) fn invariant_failure(&mut self, message: impl Into<String>) -> AvroDiagnostic {
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

    pub(super) fn type_suffix(&self, tpe: &TypeExpr) -> String {
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

    pub(super) fn mapped_type(
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

    pub(super) fn project_tuple(
        &mut self,
        schema_namespace: &str,
        owner_source: &str,
        elements: &[TypeExpr],
    ) -> Result<AvroType, AvroDiagnostic> {
        let projected_elements = elements
            .iter()
            .map(|element| self.project_type(schema_namespace, owner_source, element))
            .collect::<Result<Vec<_>, _>>()?;
        let identity = canonical_tuple_identity(&projected_elements);
        let digest = Sha256::digest(&identity);
        let prefix = digest[..6]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let full_name = AvroFullName::new(schema_namespace.to_owned(), format!("Tuple_{prefix}"))?;
        self.registry
            .claim_bytes(&full_name.to_string(), &identity)?;
        if !self.contains_schema(&full_name) {
            let fields = projected_elements
                .into_iter()
                .enumerate()
                .map(|(index, element)| {
                    AvroField::new(format!("item{}", index + 1), element, Properties::new())
                })
                .collect::<Result<Vec<_>, _>>()?;
            self.insert(
                NamedSchema::Record(RecordSchema::new(
                    full_name.clone(),
                    fields,
                    None,
                    BTreeMap::from([("morphir.type".to_owned(), json!("Tuple"))]),
                )?),
                self.ownership_for_source(owner_source),
            );
        }
        Ok(AvroType::Named(full_name))
    }
}

pub(super) fn validate_physical_mappings(options: &AvroOptions) -> Result<(), AvroDiagnostic> {
    options
        .type_mappings
        .iter()
        .try_for_each(|(source_name, mapping)| {
            physical_type(source_name, &mapping.physical_type)
                .map(|_| ())
                .map_err(|error| error.with_source(source_name))
        })
}

pub(super) fn physical_type(
    source_name: &str,
    physical_type: &str,
) -> Result<AvroType, AvroDiagnostic> {
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

pub(super) fn source_properties(source_name: &str) -> Properties {
    BTreeMap::from([(
        "morphir.fqname".to_owned(),
        Value::String(source_name.to_owned()),
    )])
}

pub(super) fn canonical_type(tpe: &TypeExpr) -> String {
    match tpe {
        TypeExpr::Variable(name) => format!("var({name})"),
        TypeExpr::Reference {
            source_name,
            arguments,
        } => format!(
            "ref({source_name};{})",
            arguments
                .iter()
                .map(canonical_type)
                .collect::<Vec<_>>()
                .join(",")
        ),
        TypeExpr::Tuple(elements) => format!(
            "tuple({})",
            elements
                .iter()
                .map(canonical_type)
                .collect::<Vec<_>>()
                .join(",")
        ),
        TypeExpr::Record(fields) => format!(
            "record({})",
            fields
                .iter()
                .map(|field| format!("{}:{}", field.name, canonical_type(&field.tpe)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        TypeExpr::ExtensibleRecord { variable, fields } => format!(
            "extensible({variable};{})",
            fields
                .iter()
                .map(|field| format!("{}:{}", field.name, canonical_type(&field.tpe)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        TypeExpr::Function { input, output } => {
            format!(
                "function({};{})",
                canonical_type(input),
                canonical_type(output)
            )
        }
        TypeExpr::Unit => "unit".to_owned(),
    }
}

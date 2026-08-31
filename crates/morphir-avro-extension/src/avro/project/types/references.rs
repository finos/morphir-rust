use super::super::*;
use super::super::{
    declarations::substitute,
    protocol::{declaration_doc, declaration_type_params},
};
use super::{canonical::type_complexity, canonical_type};

impl Projector<'_> {
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
        let complexity = arguments.iter().map(type_complexity).sum();
        if let Some(active) = self.active_declarations.get(source_name) {
            if active
                .iter()
                .any(|specialization| specialization.arguments == arguments)
                && is_named
            {
                return Ok(AvroType::Named(full_name));
            }
            if active.last().is_some_and(|specialization| {
                complexity > specialization.complexity
                    || (complexity == specialization.complexity && !is_named)
            }) {
                return Err(AvroDiagnostic::unsafe_recursion(source_name));
            }
        }
        self.active_declarations
            .entry(source_name.to_owned())
            .or_default()
            .push(ActiveSpecialization {
                arguments: arguments.to_vec(),
                complexity,
            });
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
        let remove_active_entry =
            self.active_declarations
                .get_mut(source_name)
                .is_some_and(|active| {
                    active.pop();
                    active.is_empty()
                });
        if remove_active_entry {
            self.active_declarations.remove(source_name);
        }
        result
    }
}

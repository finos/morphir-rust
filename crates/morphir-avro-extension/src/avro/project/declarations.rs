use super::*;
use super::{
    encoding::type_application_digest,
    protocol::declaration_doc,
    types::{canonical_type, source_properties},
};

impl Projector<'_> {
    pub(super) fn project_declaration(
        &mut self,
        package_name: &str,
        module: &ProjectionModule,
        declaration: &TypeDeclaration,
    ) -> Result<(), AvroDiagnostic> {
        // Performance boundary: root transactionality currently clones all
        // accumulated projection maps once per artifact. Replace this with
        // bounded staging maps if profiling shows quadratic package growth.
        let mut scratch = self.clone();
        match scratch.project_declaration_inner(package_name, module, declaration) {
            Ok(()) => {
                *self = scratch;
                Ok(())
            }
            Err(error) => Err(error.with_source(declaration.source_name())),
        }
    }

    pub(super) fn project_declaration_inner(
        &mut self,
        package_name: &str,
        module: &ProjectionModule,
        declaration: &TypeDeclaration,
    ) -> Result<(), AvroDiagnostic> {
        let full_name = AvroFullName::new(
            namespace(package_name, &module.path),
            upper_camel(declaration.name()),
        )?;
        let doc = declaration_doc(declaration);
        if let Some(mapping) = self.options.type_mappings.get(declaration.source_name()) {
            let tpe = self.mapped_type(declaration.source_name(), mapping)?;
            self.insert_root(declaration.source_name(), full_name, tpe, doc)?;
            return Ok(());
        }
        match declaration {
            TypeDeclaration::Alias { type_params, .. }
            | TypeDeclaration::Custom { type_params, .. }
                if !type_params.is_empty() =>
            {
                let Some(parameter) = type_params.first() else {
                    return Err(self.invariant_failure(
                        "generic declaration guard accepted an empty parameter list",
                    ));
                };
                Err(AvroDiagnostic::unbound_type_parameter(format!(
                    "{parameter} at {}",
                    declaration.source_name()
                )))
            }
            TypeDeclaration::Alias {
                source_name, value, ..
            } => {
                if matches!(value, TypeExpr::Record(_))
                    || self.options.aliases == Aliases::WrapperRecord
                {
                    self.project_alias_schema(
                        source_name,
                        &full_name,
                        value,
                        doc,
                        SchemaOwnership::Owned,
                    )?;
                    self.insert_root(
                        declaration.source_name(),
                        full_name.clone(),
                        AvroType::Named(full_name),
                        doc,
                    )?;
                } else {
                    let tpe = self.project_type(full_name.namespace(), source_name, value)?;
                    self.insert_root(declaration.source_name(), full_name, tpe, doc)?;
                }
                Ok(())
            }
            TypeDeclaration::Custom {
                source_name,
                constructors,
                ..
            } => {
                self.project_custom(
                    source_name,
                    &full_name,
                    constructors,
                    &BTreeMap::new(),
                    doc,
                    SchemaOwnership::Owned,
                )?;
                self.insert_root(
                    declaration.source_name(),
                    full_name.clone(),
                    AvroType::Named(full_name),
                    doc,
                )?;
                Ok(())
            }
            TypeDeclaration::Opaque { source_name, .. }
            | TypeDeclaration::Incomplete { source_name, .. } => {
                Err(AvroDiagnostic::unsupported_morphir_type(source_name))
            }
        }
    }

    pub(super) fn project_fields(
        &mut self,
        schema_namespace: &str,
        owner_source: &str,
        fields: &[NamedType],
    ) -> Result<Vec<AvroField>, AvroDiagnostic> {
        let mut projected = fields
            .iter()
            .map(|field| {
                AvroField::new(
                    lower_camel(&field.name),
                    self.project_type(schema_namespace, owner_source, &field.tpe)?,
                    Properties::new(),
                )
            })
            .collect::<Result<Vec<_>, AvroDiagnostic>>()?;
        projected.sort_by(|left, right| left.name().cmp(right.name()));
        for pair in projected.windows(2) {
            if pair[0].name() == pair[1].name() {
                return Err(AvroDiagnostic::name_collision(pair[0].name()));
            }
        }
        Ok(projected)
    }

    pub(super) fn project_alias_schema(
        &mut self,
        source_name: &str,
        full_name: &AvroFullName,
        value: &TypeExpr,
        doc: Option<&str>,
        ownership: SchemaOwnership,
    ) -> Result<(), AvroDiagnostic> {
        if self.contains_schema(full_name) {
            return Ok(());
        }
        if !self.building_schemas.insert(full_name.to_string()) {
            return Ok(());
        }
        let fields = match value {
            TypeExpr::Record(fields) => {
                self.project_fields(full_name.namespace(), source_name, fields)?
            }
            other => vec![AvroField::new(
                "value".to_owned(),
                self.project_type(full_name.namespace(), source_name, other)?,
                Properties::new(),
            )?],
        };
        self.insert(
            NamedSchema::Record(RecordSchema::new(
                full_name.clone(),
                fields,
                doc.map(str::to_owned),
                source_properties(source_name),
            )?),
            ownership,
        );
        self.building_schemas.remove(&full_name.to_string());
        Ok(())
    }

    pub(super) fn project_custom(
        &mut self,
        source_name: &str,
        full_name: &AvroFullName,
        constructors: &[Constructor],
        substitutions: &BTreeMap<String, TypeExpr>,
        doc: Option<&str>,
        ownership: SchemaOwnership,
    ) -> Result<(), AvroDiagnostic> {
        if self.contains_schema(full_name) {
            return Ok(());
        }
        if !self.building_schemas.insert(full_name.to_string()) {
            return Ok(());
        }
        if !constructors.is_empty()
            && constructors
                .iter()
                .all(|constructor| constructor.arguments.is_empty())
        {
            let symbols = constructors
                .iter()
                .map(|constructor| upper_camel(&constructor.name))
                .collect();
            self.insert(
                NamedSchema::Enum(EnumSchema::new(
                    full_name.clone(),
                    symbols,
                    doc.map(str::to_owned),
                    source_properties(source_name),
                )?),
                ownership,
            );
            self.building_schemas.remove(&full_name.to_string());
            return Ok(());
        }
        if constructors.is_empty() {
            return Err(AvroDiagnostic::unsupported_morphir_type(source_name));
        }
        let mut constructors = constructors.iter().collect::<Vec<_>>();
        constructors.sort_by_key(|constructor| upper_camel(&constructor.name));
        let mut branches = Vec::with_capacity(constructors.len());
        for constructor in constructors {
            let constructor_name =
                AvroFullName::new(full_name.to_string(), upper_camel(&constructor.name))?;
            self.registry.claim(
                &constructor_name.to_string(),
                &format!("{}:{}", constructor.source_name, full_name),
            )?;
            let arguments = constructor
                .arguments
                .iter()
                .map(|argument| NamedType {
                    name: argument.name.clone(),
                    tpe: substitute(&argument.tpe, substitutions),
                })
                .collect::<Vec<_>>();
            let fields =
                self.project_fields(constructor_name.namespace(), source_name, &arguments)?;
            self.insert(
                NamedSchema::Record(RecordSchema::new(
                    constructor_name.clone(),
                    fields,
                    None,
                    source_properties(&constructor.source_name),
                )?),
                ownership,
            );
            branches.push(AvroType::Named(constructor_name));
        }
        let union = AvroUnion::new(branches).map_err(AvroDiagnostic::unsupported_morphir_type)?;
        self.insert(
            NamedSchema::Record(RecordSchema::new(
                full_name.clone(),
                vec![AvroField::new(
                    "value".to_owned(),
                    AvroType::Union(union),
                    Properties::new(),
                )?],
                doc.map(str::to_owned),
                source_properties(source_name),
            )?),
            ownership,
        );
        self.building_schemas.remove(&full_name.to_string());
        Ok(())
    }

    pub(super) fn project_result(
        &mut self,
        schema_namespace: &str,
        owner_source: &str,
        error: &TypeExpr,
        value: &TypeExpr,
    ) -> Result<AvroType, AvroDiagnostic> {
        let full_name = AvroFullName::new(
            schema_namespace.to_owned(),
            format!(
                "Result{}{}_{}",
                self.type_suffix(error),
                self.type_suffix(value),
                type_application_digest(SDK_RESULT, &[error.clone(), value.clone()])
            ),
        )?;
        self.registry.claim(
            &full_name.to_string(),
            &format!(
                "{SDK_RESULT}<{};{}>",
                canonical_type(error),
                canonical_type(value)
            ),
        )?;
        let constructors = vec![
            Constructor {
                source_name: "morphir/SDK:result#err".to_owned(),
                name: "err".to_owned(),
                arguments: vec![NamedType {
                    name: "error".to_owned(),
                    tpe: error.clone(),
                }],
            },
            Constructor {
                source_name: "morphir/SDK:result#ok".to_owned(),
                name: "ok".to_owned(),
                arguments: vec![NamedType {
                    name: "value".to_owned(),
                    tpe: value.clone(),
                }],
            },
        ];
        self.project_custom(
            SDK_RESULT,
            &full_name,
            &constructors,
            &BTreeMap::new(),
            None,
            self.ownership_for_source(owner_source),
        )?;
        Ok(AvroType::Named(full_name))
    }
}

pub(super) fn substitute(tpe: &TypeExpr, substitutions: &BTreeMap<String, TypeExpr>) -> TypeExpr {
    match tpe {
        TypeExpr::Variable(name) => substitutions
            .get(name)
            .cloned()
            .unwrap_or_else(|| tpe.clone()),
        TypeExpr::Reference {
            source_name,
            arguments,
        } => TypeExpr::Reference {
            source_name: source_name.clone(),
            arguments: arguments
                .iter()
                .map(|argument| substitute(argument, substitutions))
                .collect(),
        },
        TypeExpr::Tuple(elements) => TypeExpr::Tuple(
            elements
                .iter()
                .map(|element| substitute(element, substitutions))
                .collect(),
        ),
        TypeExpr::Record(fields) => TypeExpr::Record(substitute_fields(fields, substitutions)),
        TypeExpr::ExtensibleRecord { variable, fields } => TypeExpr::ExtensibleRecord {
            variable: variable.clone(),
            fields: substitute_fields(fields, substitutions),
        },
        TypeExpr::Function { input, output } => TypeExpr::Function {
            input: Box::new(substitute(input, substitutions)),
            output: Box::new(substitute(output, substitutions)),
        },
        TypeExpr::Unit => TypeExpr::Unit,
    }
}

pub(super) fn substitute_fields(
    fields: &[NamedType],
    substitutions: &BTreeMap<String, TypeExpr>,
) -> Vec<NamedType> {
    fields
        .iter()
        .map(|field| NamedType {
            name: field.name.clone(),
            tpe: substitute(&field.tpe, substitutions),
        })
        .collect()
}

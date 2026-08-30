use std::collections::{BTreeMap, BTreeSet};

use morphir_extension_sdk::DiagnosticSeverity;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{
    AvroField, AvroFullName, AvroMessage, AvroPackage, AvroRequest, AvroRoot, AvroType, AvroUnion,
    EnumSchema, NamedSchema, Properties, Protocol, RecordSchema,
};
use crate::{
    Aliases, AvroDiagnostic, AvroGenerationError, AvroInternalError, AvroOptions, Constructor,
    Dependencies, DistributionKind, EntryPointKind, NamedType, ProjectedDiagnostic, Projection,
    ProjectionModule, ProjectionPackage, TypeDeclaration, TypeExpr, Unsupported, ValueKind,
    ValueSpecification,
    naming::{NameRegistry, full_name_from_source, lower_camel, namespace, upper_camel},
};

const SDK_BOOL: &str = "morphir/SDK:basics#bool";
const SDK_INT: &str = "morphir/SDK:basics#int";
const SDK_FLOAT: &str = "morphir/SDK:basics#float";
const SDK_STRING: &str = "morphir/SDK:string#string";
const SDK_CHAR: &str = "morphir/SDK:char#char";
const SDK_MAYBE: &str = "morphir/SDK:maybe#maybe";
const SDK_LIST: &str = "morphir/SDK:list#list";
const SDK_SET: &str = "morphir/SDK:set#set";
const SDK_DICT: &str = "morphir/SDK:dict#dict";
const SDK_RESULT: &str = "morphir/SDK:result#result";
const SDK_LOCAL_DATE: &str = "morphir/SDK:local-date#local-date";
const SDK_LOCAL_TIME: &str = "morphir/SDK:local-time#local-time";
const SDK_INSTANT: &str = "morphir/SDK:instant#instant";
const SDK_DATE_TIME: &str = "morphir/SDK:date-time#date-time";
const SDK_UUID: &str = "morphir/SDK:uuid#uuid";
const SDK_DECIMAL: &str = "morphir/SDK:decimal#decimal";

/// Project a normalized, body-free Morphir package into the semantic Avro model.
///
/// The returned roots retain every supported public type, including aliases
/// whose Avro representation is not itself a named schema.
///
/// # Examples
///
/// ```
/// use morphir_avro_extension::{
///     AvroOptions, DistributionKind, ProjectionPackage, project,
/// };
///
/// let package = ProjectionPackage {
///     kind: DistributionKind::Library,
///     package_name: "example".to_owned(),
///     dependencies: Vec::new(),
///     modules: Vec::new(),
/// };
/// let avro = project(&package, &AvroOptions::default())?;
/// assert!(avro.roots().is_empty());
/// # Ok::<(), morphir_avro_extension::AvroGenerationError>(())
/// ```
pub fn project(
    package: &ProjectionPackage,
    options: &AvroOptions,
) -> Result<AvroPackage, AvroGenerationError> {
    options.validate().map_err(AvroGenerationError::from)?;
    validate_physical_mappings(options).map_err(AvroGenerationError::from)?;
    let mut projector = Projector::new(options);
    let mut diagnostics = projector.register_package(package);
    diagnostics.extend(projector.project_modules(&package.package_name, &package.modules));
    diagnostics.extend(projector.project_protocols(
        package.kind,
        &package.package_name,
        &package.modules,
    ));
    if let Some(error) = projector.internal_failure.take() {
        return Err(error.into());
    }
    sort_diagnostics(&mut diagnostics);
    if options.unsupported == Unsupported::Error && !diagnostics.is_empty() {
        return Err(diagnostics.into());
    }
    let diagnostics = diagnostics
        .into_iter()
        .map(|diagnostic| ProjectedDiagnostic::new(diagnostic, DiagnosticSeverity::Warning))
        .collect();
    projector.finish(diagnostics).map_err(Into::into)
}

#[derive(Clone)]
struct Projector<'options> {
    options: &'options AvroOptions,
    registry: NameRegistry,
    roots: BTreeMap<String, AvroRoot>,
    schemas: BTreeMap<String, NamedSchema>,
    linked_schemas: BTreeMap<String, NamedSchema>,
    protocols: BTreeMap<String, Protocol>,
    declarations: BTreeMap<String, DeclarationInfo>,
    invalid_declarations: BTreeSet<String>,
    active_declarations: BTreeMap<String, String>,
    building_schemas: BTreeSet<String>,
    internal_failure: Option<AvroInternalError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchemaOwnership {
    Owned,
    Linked,
}

#[derive(Clone)]
struct DeclarationInfo {
    declaration: TypeDeclaration,
    full_name: AvroFullName,
    dependency: bool,
}

struct DeclarationCandidate {
    declaration: TypeDeclaration,
    full_name: AvroFullName,
    dependency: bool,
}

fn declaration_candidates(
    package_name: &str,
    modules: &[ProjectionModule],
    dependency: bool,
) -> (Vec<DeclarationCandidate>, Vec<AvroDiagnostic>) {
    let mut candidates = Vec::new();
    let mut diagnostics = Vec::new();
    for module in modules {
        for declaration in &module.types {
            match AvroFullName::new(
                namespace(package_name, &module.path),
                upper_camel(declaration.name()),
            ) {
                Ok(full_name) => candidates.push(DeclarationCandidate {
                    declaration: declaration.clone(),
                    full_name,
                    dependency,
                }),
                Err(error) => diagnostics.push(error.with_source(declaration.source_name())),
            }
        }
    }
    (candidates, diagnostics)
}

impl<'options> Projector<'options> {
    fn new(options: &'options AvroOptions) -> Self {
        Self {
            options,
            registry: NameRegistry::default(),
            roots: BTreeMap::new(),
            schemas: BTreeMap::new(),
            linked_schemas: BTreeMap::new(),
            protocols: BTreeMap::new(),
            declarations: BTreeMap::new(),
            invalid_declarations: BTreeSet::new(),
            active_declarations: BTreeMap::new(),
            building_schemas: BTreeSet::new(),
            internal_failure: None,
        }
    }

    fn register_package(&mut self, package: &ProjectionPackage) -> Vec<AvroDiagnostic> {
        let (mut candidates, mut diagnostics) =
            declaration_candidates(&package.package_name, &package.modules, false);
        for dependency in &package.dependencies {
            let (dependency_candidates, dependency_diagnostics) =
                declaration_candidates(&dependency.package_name, &dependency.modules, true);
            candidates.extend(dependency_candidates);
            diagnostics.extend(dependency_diagnostics);
        }
        self.invalid_declarations.extend(
            diagnostics
                .iter()
                .filter_map(|diagnostic| diagnostic.source().map(str::to_owned)),
        );
        candidates.sort_by(|left, right| {
            left.declaration
                .source_name()
                .cmp(right.declaration.source_name())
                .then_with(|| left.full_name.to_string().cmp(&right.full_name.to_string()))
                .then_with(|| left.dependency.cmp(&right.dependency))
        });

        let mut by_source = BTreeMap::<String, Vec<&DeclarationCandidate>>::new();
        let mut by_full_name = BTreeMap::<String, Vec<&DeclarationCandidate>>::new();
        for candidate in &candidates {
            by_source
                .entry(candidate.declaration.source_name().to_owned())
                .or_default()
                .push(candidate);
            by_full_name
                .entry(candidate.full_name.to_string())
                .or_default()
                .push(candidate);
        }
        for (source, conflicts) in &by_source {
            if conflicts.len() <= 1 {
                continue;
            }
            self.invalid_declarations.insert(source.clone());
            for conflict in conflicts {
                diagnostics.push(
                    AvroDiagnostic::name_collision(format!(
                        "duplicate Morphir source {source} at {}",
                        conflict.full_name
                    ))
                    .with_source(source),
                );
            }
        }
        for (full_name, conflicts) in &by_full_name {
            let sources = conflicts
                .iter()
                .map(|candidate| candidate.declaration.source_name())
                .collect::<BTreeSet<_>>();
            if sources.len() <= 1 {
                continue;
            }
            for source in sources {
                self.invalid_declarations.insert(source.to_owned());
                diagnostics
                    .push(AvroDiagnostic::name_collision(full_name).with_source(source.to_owned()));
            }
        }
        for candidate in &candidates {
            let TypeDeclaration::Custom { constructors, .. } = &candidate.declaration else {
                continue;
            };
            let mut by_name = BTreeMap::<String, Vec<&Constructor>>::new();
            for constructor in constructors {
                by_name
                    .entry(upper_camel(&constructor.name))
                    .or_default()
                    .push(constructor);
            }
            for (name, conflicts) in by_name {
                if conflicts.len() <= 1 {
                    continue;
                }
                self.invalid_declarations
                    .insert(candidate.declaration.source_name().to_owned());
                for constructor in conflicts {
                    diagnostics.push(
                        AvroDiagnostic::name_collision(format!("{}.{}", candidate.full_name, name))
                            .with_source(&constructor.source_name),
                    );
                }
            }
        }

        for candidate in candidates {
            let source = candidate.declaration.source_name().to_owned();
            if self.invalid_declarations.contains(&source) {
                continue;
            }
            if let Err(error) = self
                .registry
                .claim(&candidate.full_name.to_string(), &source)
            {
                self.invalid_declarations.insert(source.clone());
                diagnostics.push(error.with_source(source));
                continue;
            }
            self.declarations.insert(
                source,
                DeclarationInfo {
                    declaration: candidate.declaration,
                    full_name: candidate.full_name,
                    dependency: candidate.dependency,
                },
            );
        }
        deduplicate_diagnostics(&mut diagnostics);
        diagnostics
    }

    fn project_modules(
        &mut self,
        package_name: &str,
        modules: &[ProjectionModule],
    ) -> Vec<AvroDiagnostic> {
        let mut diagnostics = Vec::new();
        let mut modules = modules.iter().collect::<Vec<_>>();
        modules.sort_by(|left, right| left.path.cmp(&right.path));
        for module in modules {
            let mut declarations = module.types.iter().collect::<Vec<_>>();
            declarations.sort_by(|left, right| left.source_name().cmp(right.source_name()));
            for declaration in declarations {
                if self
                    .invalid_declarations
                    .contains(declaration.source_name())
                {
                    continue;
                }
                if let Err(error) = self.project_declaration(package_name, module, declaration) {
                    diagnostics.push(error);
                }
            }
        }
        diagnostics
    }

    fn project_protocols(
        &mut self,
        distribution_kind: DistributionKind,
        package_name: &str,
        modules: &[ProjectionModule],
    ) -> Vec<AvroDiagnostic> {
        if self.options.projection == Projection::Schemas {
            return Vec::new();
        }
        let mut diagnostics = Vec::new();
        let mut candidates = Vec::new();
        for module in modules {
            let source = module_source(package_name, module);
            let (protocol_namespace, protocol_name) = protocol_identity(package_name, &module.path);
            match AvroFullName::new(protocol_namespace, protocol_name) {
                Ok(full_name) => candidates.push((source, full_name, module)),
                Err(error) => diagnostics.push(error.with_source(source)),
            }
        }
        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        let mut by_full_name = BTreeMap::<String, Vec<&str>>::new();
        for (source, full_name, _) in &candidates {
            by_full_name
                .entry(full_name.to_string())
                .or_default()
                .push(source);
        }
        let mut quarantined = BTreeSet::new();
        for (full_name, sources) in by_full_name {
            if sources.len() <= 1 {
                continue;
            }
            for source in sources {
                quarantined.insert(source.to_owned());
                diagnostics.push(
                    AvroDiagnostic::name_collision(&full_name).with_source(source.to_owned()),
                );
            }
        }
        for (source, full_name, module) in candidates {
            if quarantined.contains(&source) {
                continue;
            }
            diagnostics.extend(self.project_protocol(
                distribution_kind,
                package_name,
                module,
                source,
                full_name,
            ));
        }
        diagnostics
    }

    fn project_protocol(
        &mut self,
        distribution_kind: DistributionKind,
        package_name: &str,
        module: &ProjectionModule,
        module_source: String,
        full_name: AvroFullName,
    ) -> Vec<AvroDiagnostic> {
        if let Err(error) = self
            .registry
            .claim(&full_name.to_string(), &format!("module:{module_source}"))
        {
            return vec![error.with_source(module_source)];
        }
        let mut selected = module
            .values
            .iter()
            .filter(|value| match self.options.projection {
                Projection::Schemas => false,
                Projection::ProtocolEntryPoints => {
                    distribution_kind == DistributionKind::Application
                        && value.entry_point.is_some()
                }
                Projection::ProtocolPublic => true,
            })
            .collect::<Vec<_>>();
        selected.sort_by(|left, right| left.source_name.cmp(&right.source_name));
        let mut by_name = BTreeMap::<String, Vec<&ValueSpecification>>::new();
        for value in &selected {
            by_name
                .entry(lower_camel(&value.name))
                .or_default()
                .push(value);
        }
        let mut quarantined = BTreeSet::new();
        let mut diagnostics = Vec::new();
        for (name, values) in by_name {
            if values.len() <= 1 {
                continue;
            }
            for value in values {
                quarantined.insert(value.source_name.clone());
                diagnostics
                    .push(AvroDiagnostic::name_collision(&name).with_source(&value.source_name));
            }
        }

        let mut messages = Vec::new();
        for value in selected {
            if quarantined.contains(&value.source_name) {
                continue;
            }
            // Performance boundary: message transactionality currently clones
            // all accumulated projection maps. A future staging-map refactor
            // must roll back registry claims, schemas, and dependency closures.
            let mut scratch = self.clone();
            match scratch.project_message(full_name.namespace(), value) {
                Ok(message) => {
                    *self = scratch;
                    messages.push(message);
                }
                Err(error) => {
                    diagnostics.push(error.with_source(&value.source_name));
                }
            }
        }
        let mut properties = Properties::from([
            ("morphir.package".to_owned(), json!(package_name)),
            ("morphir.module".to_owned(), json!(module.path.join("/"))),
        ]);
        if let Some(doc) = &module.doc {
            properties.insert("morphir.doc".to_owned(), json!(doc));
        }
        let type_roots = module
            .types
            .iter()
            .filter_map(|declaration| {
                self.roots
                    .values()
                    .find(|root| root.source_fqname() == declaration.source_name())
                    .map(|root| root.tpe().clone())
            })
            .collect();
        match Protocol::new(full_name.clone(), messages, type_roots, properties) {
            Ok(protocol) => {
                self.protocols.insert(full_name.to_string(), protocol);
            }
            Err(error) => diagnostics.push(error.with_source(module_source)),
        }
        diagnostics
    }

    fn project_message(
        &mut self,
        protocol_namespace: &str,
        value: &ValueSpecification,
    ) -> Result<AvroMessage, AvroDiagnostic> {
        if value.value_kind == ValueKind::Constant && !value.inputs.is_empty() {
            return Err(AvroDiagnostic::unsupported_morphir_type(format!(
                "constant {} declares request inputs",
                value.source_name
            ))
            .with_source(&value.source_name));
        }
        let output = value.output.as_ref().ok_or_else(|| {
            AvroDiagnostic::unsupported_morphir_type(format!(
                "value {} has no output type",
                value.source_name
            ))
            .with_source(&value.source_name)
        })?;
        let request = AvroRequest::new(
            self.project_fields(protocol_namespace, &value.source_name, &value.inputs)
                .map_err(|error| error.with_source(&value.source_name))?,
        )
        .map_err(|error| error.with_source(&value.source_name))?;
        let response = self
            .project_type(protocol_namespace, &value.source_name, output)
            .map_err(|error| error.with_source(&value.source_name))?;
        let mut properties = Properties::from([
            ("morphir.fqname".to_owned(), json!(value.source_name)),
            (
                "morphir.value-kind".to_owned(),
                json!(match value.value_kind {
                    ValueKind::Constant => "constant",
                    ValueKind::Function => "function",
                }),
            ),
        ]);
        if let Some(doc) = &value.doc {
            properties.insert("morphir.doc".to_owned(), json!(doc));
        }
        if let Some(entry_point) = &value.entry_point {
            properties.insert("morphir.entry-point".to_owned(), json!(true));
            properties.insert(
                "morphir.entry-point-kind".to_owned(),
                json!(entry_point_kind(entry_point.kind)),
            );
            properties.insert(
                "morphir.entry-point-id".to_owned(),
                json!(entry_point.identifier),
            );
            if let Some(doc) = &entry_point.doc {
                properties.insert("morphir.entry-point-doc".to_owned(), json!(doc));
            }
        }
        AvroMessage::new(
            lower_camel(&value.name),
            request,
            response,
            Vec::new(),
            properties,
        )
        .map_err(|error| error.with_source(&value.source_name))
    }

    fn project_declaration(
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

    fn project_declaration_inner(
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

    fn project_fields(
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

    fn project_type(
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

    fn project_reference(
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
                if !is_sdk_string(&arguments[0]) {
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

    fn project_declared_reference(
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

    fn invariant_failure(&mut self, message: impl Into<String>) -> AvroDiagnostic {
        let error = AvroInternalError::invariant(message);
        if self.internal_failure.is_none() {
            self.internal_failure = Some(error);
        }
        AvroDiagnostic::unsupported_morphir_type("internal projection invariant")
    }

    fn project_alias_schema(
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

    fn project_custom(
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

    fn project_result(
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

    fn specialized_name(
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

    fn type_suffix(&self, tpe: &TypeExpr) -> String {
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

    fn logical_type(&self, physical: AvroType, name: &str, properties: Properties) -> AvroType {
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

    fn mapped_type(
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

    fn project_tuple(
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

    fn insert(&mut self, schema: NamedSchema, ownership: SchemaOwnership) {
        if ownership == SchemaOwnership::Owned {
            self.schemas.insert(schema.full_name().to_string(), schema);
        } else {
            self.linked_schemas
                .insert(schema.full_name().to_string(), schema);
        }
    }

    fn contains_schema(&self, full_name: &AvroFullName) -> bool {
        let full_name = full_name.to_string();
        self.schemas.contains_key(&full_name) || self.linked_schemas.contains_key(&full_name)
    }

    fn declaration_ownership(&self, info: &DeclarationInfo) -> SchemaOwnership {
        if info.dependency && self.options.dependencies == Dependencies::Linked {
            SchemaOwnership::Linked
        } else {
            SchemaOwnership::Owned
        }
    }

    fn ownership_for_source(&self, source_name: &str) -> SchemaOwnership {
        self.declarations
            .get(source_name)
            .map(|info| self.declaration_ownership(info))
            .unwrap_or(SchemaOwnership::Owned)
    }

    fn insert_root(
        &mut self,
        source_fqname: &str,
        full_name: AvroFullName,
        tpe: AvroType,
        doc: Option<&str>,
    ) -> Result<(), AvroDiagnostic> {
        self.roots.insert(
            full_name.to_string(),
            AvroRoot::new(
                source_fqname.to_owned(),
                full_name,
                tpe,
                doc.map(str::to_owned),
            )?,
        );
        Ok(())
    }

    fn finish(self, diagnostics: Vec<ProjectedDiagnostic>) -> Result<AvroPackage, AvroDiagnostic> {
        AvroPackage::new(
            self.roots.into_values().collect(),
            self.schemas.into_values().collect(),
            self.linked_schemas.into_values().collect(),
            self.protocols.into_values().collect(),
            diagnostics,
        )
    }
}

fn sort_diagnostics(diagnostics: &mut [AvroDiagnostic]) {
    diagnostics.sort_by(|left, right| {
        left.source()
            .unwrap_or("")
            .cmp(right.source().unwrap_or(""))
            .then_with(|| left.code().cmp(right.code()))
            .then_with(|| left.message().cmp(right.message()))
    });
}

fn deduplicate_diagnostics(diagnostics: &mut Vec<AvroDiagnostic>) {
    sort_diagnostics(diagnostics);
    diagnostics.dedup_by(|left, right| {
        left.source() == right.source()
            && left.code() == right.code()
            && left.message() == right.message()
    });
}

fn protocol_identity(package_name: &str, module_path: &[String]) -> (String, String) {
    match module_path.split_last() {
        Some((name, parents)) => (namespace(package_name, parents), upper_camel(name)),
        None => {
            let name = package_name.rsplit('/').next().unwrap_or("Protocol");
            (namespace(package_name, &[]), upper_camel(name))
        }
    }
}

fn module_source(package_name: &str, module: &ProjectionModule) -> String {
    format!("{package_name}:{}", module.path.join("/"))
}

fn entry_point_kind(kind: EntryPointKind) -> &'static str {
    match kind {
        EntryPointKind::Main => "main",
        EntryPointKind::Command => "command",
        EntryPointKind::Handler => "handler",
    }
}

fn declaration_type_params(declaration: &TypeDeclaration) -> &[String] {
    match declaration {
        TypeDeclaration::Alias { type_params, .. }
        | TypeDeclaration::Opaque { type_params, .. }
        | TypeDeclaration::Custom { type_params, .. }
        | TypeDeclaration::Incomplete { type_params, .. } => type_params,
    }
}

fn declaration_doc(declaration: &TypeDeclaration) -> Option<&str> {
    match declaration {
        TypeDeclaration::Alias { doc, .. }
        | TypeDeclaration::Opaque { doc, .. }
        | TypeDeclaration::Custom { doc, .. }
        | TypeDeclaration::Incomplete { doc, .. } => doc.as_deref(),
    }
}

fn substitute(tpe: &TypeExpr, substitutions: &BTreeMap<String, TypeExpr>) -> TypeExpr {
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

fn substitute_fields(
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

fn validate_physical_mappings(options: &AvroOptions) -> Result<(), AvroDiagnostic> {
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

fn type_application_digest(source_name: &str, arguments: &[TypeExpr]) -> String {
    let mut identity = vec![b'G'];
    encode_string(&mut identity, source_name);
    encode_len(&mut identity, arguments.len());
    for argument in arguments {
        encode_type_expr(&mut identity, argument);
    }
    Sha256::digest(identity)[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn encode_type_expr(output: &mut Vec<u8>, tpe: &TypeExpr) {
    match tpe {
        TypeExpr::Variable(name) => {
            output.push(b'v');
            encode_string(output, name);
        }
        TypeExpr::Reference {
            source_name,
            arguments,
        } => {
            output.push(b'r');
            encode_string(output, source_name);
            encode_len(output, arguments.len());
            for argument in arguments {
                encode_type_expr(output, argument);
            }
        }
        TypeExpr::Tuple(elements) => {
            output.push(b't');
            encode_len(output, elements.len());
            for element in elements {
                encode_type_expr(output, element);
            }
        }
        TypeExpr::Record(fields) => {
            output.push(b'c');
            encode_type_fields(output, fields);
        }
        TypeExpr::ExtensibleRecord { variable, fields } => {
            output.push(b'e');
            encode_string(output, variable);
            encode_type_fields(output, fields);
        }
        TypeExpr::Function {
            input,
            output: result,
        } => {
            output.push(b'f');
            encode_type_expr(output, input);
            encode_type_expr(output, result);
        }
        TypeExpr::Unit => output.push(b'u'),
    }
}

fn encode_type_fields(output: &mut Vec<u8>, fields: &[NamedType]) {
    encode_len(output, fields.len());
    for field in fields {
        encode_string(output, &field.name);
        encode_type_expr(output, &field.tpe);
    }
}

/// Encode projected Avro types for stable synthetic-name hashing.
///
/// Every node starts with a one-byte tag. Variable-length values use an
/// unsigned 64-bit big-endian byte length followed by raw UTF-8 bytes. Maps
/// are key-sorted, and JSON values recursively use explicit scalar, array, and
/// sorted-object tags. This encoding is independent of Rust formatting traits.
fn canonical_tuple_identity(elements: &[AvroType]) -> Vec<u8> {
    let mut output = vec![b'T'];
    encode_len(&mut output, elements.len());
    for element in elements {
        encode_avro_type(&mut output, element);
    }
    output
}

fn encode_avro_type(output: &mut Vec<u8>, tpe: &AvroType) {
    match tpe {
        AvroType::Null => output.push(b'n'),
        AvroType::Boolean => output.push(b'b'),
        AvroType::Int => output.push(b'i'),
        AvroType::Long => output.push(b'l'),
        AvroType::Float => output.push(b'f'),
        AvroType::Double => output.push(b'd'),
        AvroType::Bytes => output.push(b'y'),
        AvroType::String => output.push(b's'),
        AvroType::Array(element, properties) => {
            output.push(b'a');
            encode_avro_type(output, element);
            encode_properties(output, properties);
        }
        AvroType::Map(value, properties) => {
            output.push(b'm');
            encode_avro_type(output, value);
            encode_properties(output, properties);
        }
        AvroType::Union(union) => {
            output.push(b'u');
            encode_len(output, union.branches().len());
            for branch in union.branches() {
                encode_avro_type(output, branch);
            }
        }
        AvroType::Named(name) => {
            output.push(b'r');
            encode_string(output, &name.to_string());
        }
        AvroType::Logical {
            physical,
            name,
            properties,
        } => {
            output.push(b'g');
            encode_avro_type(output, physical);
            encode_string(output, name);
            encode_properties(output, properties);
        }
        AvroType::Annotated {
            physical,
            properties,
        } => {
            output.push(b't');
            encode_avro_type(output, physical);
            encode_properties(output, properties);
        }
    }
}

fn encode_properties(output: &mut Vec<u8>, properties: &Properties) {
    encode_len(output, properties.len());
    for (key, value) in properties {
        encode_string(output, key);
        encode_json(output, value);
    }
}

fn encode_json(output: &mut Vec<u8>, value: &Value) {
    match value {
        Value::Null => output.push(b'0'),
        Value::Bool(value) => output.extend_from_slice(if *value { b"b1" } else { b"b0" }),
        Value::Number(value) => {
            output.push(b'd');
            encode_string(output, &value.to_string());
        }
        Value::String(value) => {
            output.push(b's');
            encode_string(output, value);
        }
        Value::Array(values) => {
            output.push(b'a');
            encode_len(output, values.len());
            for value in values {
                encode_json(output, value);
            }
        }
        Value::Object(values) => {
            output.push(b'o');
            encode_len(output, values.len());
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            for (key, value) in entries {
                encode_string(output, key);
                encode_json(output, value);
            }
        }
    }
}

fn encode_string(output: &mut Vec<u8>, value: &str) {
    encode_len(output, value.len());
    output.extend_from_slice(value.as_bytes());
}

fn encode_len(output: &mut Vec<u8>, value: usize) {
    output.extend_from_slice(&(value as u64).to_be_bytes());
}

fn source_properties(source_name: &str) -> Properties {
    BTreeMap::from([(
        "morphir.fqname".to_owned(),
        Value::String(source_name.to_owned()),
    )])
}

fn is_sdk_string(tpe: &TypeExpr) -> bool {
    matches!(
        tpe,
        TypeExpr::Reference { source_name, arguments }
            if source_name == SDK_STRING && arguments.is_empty()
    )
}

fn canonical_type(tpe: &TypeExpr) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_type_bytes_sort_nested_json_objects_and_encode_arrays_and_scalars() {
        let first = AvroType::Annotated {
            physical: Box::new(AvroType::String),
            properties: BTreeMap::from([(
                "metadata".to_owned(),
                json!({"z": [true, null, 4], "a": {"y": "two", "x": "one"}}),
            )]),
        };
        let second = AvroType::Annotated {
            physical: Box::new(AvroType::String),
            properties: BTreeMap::from([(
                "metadata".to_owned(),
                json!({"a": {"x": "one", "y": "two"}, "z": [true, null, 4]}),
            )]),
        };

        assert_eq!(
            canonical_tuple_identity(&[first]),
            canonical_tuple_identity(&[second])
        );
    }

    #[test]
    fn type_application_digest_has_a_stable_exact_encoding() {
        assert_eq!(
            type_application_digest(
                SDK_RESULT,
                &[
                    TypeExpr::Reference {
                        source_name: SDK_STRING.to_owned(),
                        arguments: Vec::new(),
                    },
                    TypeExpr::Reference {
                        source_name: "acme/one:domain#customer".to_owned(),
                        arguments: Vec::new(),
                    },
                ],
            ),
            "d920df848bb1"
        );
    }

    #[test]
    fn failed_declaration_projection_rolls_back_all_scratch_state() {
        let bad = TypeDeclaration::Alias {
            source_name: "acme/customer:domain#bad".to_owned(),
            name: "bad".to_owned(),
            type_params: Vec::new(),
            value: TypeExpr::Record(vec![
                NamedType {
                    name: "tuple".to_owned(),
                    tpe: TypeExpr::Tuple(vec![TypeExpr::Unit, TypeExpr::Unit]),
                },
                NamedType {
                    name: "unsupported".to_owned(),
                    tpe: TypeExpr::Function {
                        input: Box::new(TypeExpr::Unit),
                        output: Box::new(TypeExpr::Unit),
                    },
                },
            ]),
            doc: None,
        };
        let good = TypeDeclaration::Alias {
            source_name: "acme/customer:domain#good".to_owned(),
            name: "good".to_owned(),
            type_params: Vec::new(),
            value: TypeExpr::Record(vec![NamedType {
                name: "value".to_owned(),
                tpe: TypeExpr::Unit,
            }]),
            doc: None,
        };
        let module = ProjectionModule {
            path: vec!["domain".to_owned()],
            types: vec![bad.clone(), good.clone()],
            values: Vec::new(),
            doc: None,
        };
        let options = AvroOptions::default();
        let mut projector = Projector::new(&options);
        assert!(
            projector
                .register_package(&ProjectionPackage {
                    kind: crate::DistributionKind::Library,
                    package_name: "acme/customer".to_owned(),
                    dependencies: Vec::new(),
                    modules: vec![module.clone()],
                })
                .is_empty()
        );
        let indexed_registry = projector.registry.clone();

        assert!(
            projector
                .project_declaration("acme/customer", &module, &bad)
                .is_err()
        );
        assert!(projector.schemas.is_empty());
        assert!(projector.roots.is_empty());
        assert!(projector.building_schemas.is_empty());
        assert!(projector.active_declarations.is_empty());
        assert_eq!(projector.registry, indexed_registry);

        projector
            .project_declaration("acme/customer", &module, &good)
            .unwrap();
        assert!(projector.schemas.contains_key("acme.customer.domain.Good"));
        assert!(!projector.schemas.keys().any(|name| name.contains("Tuple_")));
    }
}

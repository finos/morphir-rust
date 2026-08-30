use super::*;

impl Projector<'_> {
    pub(super) fn project_modules(
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

    pub(super) fn project_protocols(
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

    pub(super) fn project_protocol(
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

    pub(super) fn project_message(
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
}

pub(super) fn protocol_identity(package_name: &str, module_path: &[String]) -> (String, String) {
    match module_path.split_last() {
        Some((name, parents)) => (namespace(package_name, parents), upper_camel(name)),
        None => {
            let name = package_name.rsplit('/').next().unwrap_or("Protocol");
            (namespace(package_name, &[]), upper_camel(name))
        }
    }
}

pub(super) fn module_source(package_name: &str, module: &ProjectionModule) -> String {
    format!("{package_name}:{}", module.path.join("/"))
}

pub(super) fn entry_point_kind(kind: EntryPointKind) -> &'static str {
    match kind {
        EntryPointKind::Main => "main",
        EntryPointKind::Command => "command",
        EntryPointKind::Handler => "handler",
    }
}

pub(super) fn declaration_type_params(declaration: &TypeDeclaration) -> &[String] {
    match declaration {
        TypeDeclaration::Alias { type_params, .. }
        | TypeDeclaration::Opaque { type_params, .. }
        | TypeDeclaration::Custom { type_params, .. }
        | TypeDeclaration::Incomplete { type_params, .. } => type_params,
    }
}

pub(super) fn declaration_doc(declaration: &TypeDeclaration) -> Option<&str> {
    match declaration {
        TypeDeclaration::Alias { doc, .. }
        | TypeDeclaration::Opaque { doc, .. }
        | TypeDeclaration::Custom { doc, .. }
        | TypeDeclaration::Incomplete { doc, .. } => doc.as_deref(),
    }
}

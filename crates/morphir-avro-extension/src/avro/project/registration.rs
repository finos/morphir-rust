use super::*;

impl<'options> Projector<'options> {
    pub(super) fn new(options: &'options AvroOptions) -> Self {
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

    pub(super) fn register_package(&mut self, package: &ProjectionPackage) -> Vec<AvroDiagnostic> {
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

    pub(super) fn insert(&mut self, schema: NamedSchema, ownership: SchemaOwnership) {
        if ownership == SchemaOwnership::Owned {
            self.schemas.insert(schema.full_name().to_string(), schema);
        } else {
            self.linked_schemas
                .insert(schema.full_name().to_string(), schema);
        }
    }

    pub(super) fn contains_schema(&self, full_name: &AvroFullName) -> bool {
        let full_name = full_name.to_string();
        self.schemas.contains_key(&full_name) || self.linked_schemas.contains_key(&full_name)
    }

    pub(super) fn declaration_ownership(&self, info: &DeclarationInfo) -> SchemaOwnership {
        if info.dependency && self.options.dependencies == Dependencies::Linked {
            SchemaOwnership::Linked
        } else {
            SchemaOwnership::Owned
        }
    }

    pub(super) fn ownership_for_source(&self, source_name: &str) -> SchemaOwnership {
        self.declarations
            .get(source_name)
            .map(|info| self.declaration_ownership(info))
            .unwrap_or(SchemaOwnership::Owned)
    }

    pub(super) fn insert_root(
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

    pub(super) fn finish(
        self,
        diagnostics: Vec<ProjectedDiagnostic>,
    ) -> Result<AvroPackage, AvroDiagnostic> {
        AvroPackage::new(
            self.roots.into_values().collect(),
            self.schemas.into_values().collect(),
            self.linked_schemas.into_values().collect(),
            self.protocols.into_values().collect(),
            diagnostics,
        )
    }
}

pub(super) fn sort_diagnostics(diagnostics: &mut [AvroDiagnostic]) {
    diagnostics.sort_by(|left, right| {
        left.source()
            .unwrap_or("")
            .cmp(right.source().unwrap_or(""))
            .then_with(|| left.code().cmp(right.code()))
            .then_with(|| left.message().cmp(right.message()))
    });
}

pub(super) fn deduplicate_diagnostics(diagnostics: &mut Vec<AvroDiagnostic>) {
    sort_diagnostics(diagnostics);
    diagnostics.dedup_by(|left, right| {
        left.source() == right.source()
            && left.code() == right.code()
            && left.message() == right.message()
    });
}

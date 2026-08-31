mod declarations;
mod graph;
mod syntax;
#[cfg(test)]
mod tests;

use self::{graph::*, syntax::*};
use std::collections::{BTreeMap, BTreeSet};

use morphir_extension_sdk::Artifact;
use serde_json::Value;

use crate::{
    AvroDiagnostic, AvroField, AvroFullName, AvroGenerationError, AvroInternalError, AvroMessage,
    AvroPackage, AvroRoot, AvroType, Dependencies, NamedSchema, Properties, Protocol,
    escape_idl_identifier,
};

/// Render a checked Avro package as deterministic Avro IDL artifacts.
///
/// Every artifact contains exactly one protocol. Schema projection wraps each
/// root in a message-free protocol whose name ends in `Schemas`. Linked named
/// declarations use separate protocol files and relative `import idl` paths.
/// Protocols with annotated messages returning named types require Avro Tools
/// 1.12.2's JavaCC compatibility reader (`idl --useJavaCC`); generated files
/// declare that requirement in a leading comment. Message-free schema wrappers
/// remain compatible with the default IDL reader and `idl2schemata`.
///
/// ```
/// use morphir_avro_extension::{
///     AvroOptions, Dependencies, DistributionKind, ProjectionPackage, project, render_idl,
/// };
///
/// let source = ProjectionPackage {
///     kind: DistributionKind::Library,
///     package_name: "example".to_owned(),
///     dependencies: Vec::new(),
///     modules: Vec::new(),
/// };
/// let package = project(&source, &AvroOptions::default())?;
/// let artifacts = render_idl(&package, Dependencies::SelfContained)?;
/// assert!(artifacts.is_empty());
/// # Ok::<(), morphir_avro_extension::AvroGenerationError>(())
/// ```
pub fn render_idl(
    package: &AvroPackage,
    dependencies: Dependencies,
) -> Result<Vec<Artifact>, AvroGenerationError> {
    IdlRenderer::new(package, dependencies)?.render()
}

struct IdlRenderer<'package> {
    package: &'package AvroPackage,
    dependencies: Dependencies,
    schemas: BTreeMap<String, &'package NamedSchema>,
    schema_protocol_names: BTreeMap<String, AvroFullName>,
    owned_names: BTreeSet<String>,
    linked_names: BTreeSet<String>,
    graph: BTreeMap<String, BTreeSet<String>>,
}

impl<'package> IdlRenderer<'package> {
    fn new(
        package: &'package AvroPackage,
        dependencies: Dependencies,
    ) -> Result<Self, AvroInternalError> {
        let schemas = package
            .schemas()
            .iter()
            .chain(package.linked_schemas())
            .map(|schema| (schema.full_name().to_string(), schema))
            .collect::<BTreeMap<_, _>>();
        let owned_names = package
            .schemas()
            .iter()
            .map(|schema| schema.full_name().to_string())
            .collect();
        let linked_names = package
            .linked_schemas()
            .iter()
            .map(|schema| schema.full_name().to_string())
            .collect();
        let graph = schemas
            .iter()
            .map(|(name, schema)| {
                let mut references = BTreeSet::new();
                collect_schema_references(schema, &mut references);
                references.retain(|reference| schemas.contains_key(reference));
                (name.clone(), references)
            })
            .collect();
        let source_names = package
            .roots()
            .iter()
            .map(AvroRoot::full_name)
            .chain(package.schemas().iter().map(NamedSchema::full_name))
            .chain(package.linked_schemas().iter().map(NamedSchema::full_name))
            .map(|name| (name.to_string(), name))
            .collect::<BTreeMap<_, _>>();
        let mut reserved_names = package
            .protocols()
            .iter()
            .map(|protocol| protocol.full_name().to_string())
            .collect::<BTreeSet<_>>();
        let mut schema_protocol_names = BTreeMap::new();
        for (source_name, name) in source_names {
            let mut candidate = next_schema_protocol_name(name)?;
            while reserved_names.contains(&candidate.to_string()) {
                candidate = next_schema_protocol_name(&candidate)?;
            }
            reserved_names.insert(candidate.to_string());
            schema_protocol_names.insert(source_name, candidate);
        }
        Ok(Self {
            package,
            dependencies,
            schemas,
            schema_protocol_names,
            owned_names,
            linked_names,
            graph,
        })
    }

    fn schema_protocol_name(
        &self,
        name: &AvroFullName,
    ) -> Result<&AvroFullName, AvroInternalError> {
        self.schema_protocol_names
            .get(&name.to_string())
            .ok_or_else(|| {
                AvroInternalError::invariant(format!(
                    "schema wrapper name was not allocated for {name}"
                ))
            })
    }

    fn render(&self) -> Result<Vec<Artifact>, AvroGenerationError> {
        if self.dependencies == Dependencies::Linked {
            self.validate_linked_graph()?;
        }
        let mut artifacts = Vec::new();
        let mut paths = BTreeSet::new();

        if self.dependencies == Dependencies::Linked {
            for schema in self.package.linked_schemas() {
                let artifact = self.render_linked_declaration(schema)?;
                insert_artifact(&mut artifacts, &mut paths, artifact)?;
            }
        }

        for root in self.package.roots() {
            if !self.package.named_root_is_carried_by_a_protocol(root) {
                let artifact = self.render_root(root)?;
                insert_artifact(&mut artifacts, &mut paths, artifact)?;
            }
        }
        for protocol in self.package.protocols() {
            let artifact = self.render_protocol(protocol)?;
            insert_artifact(&mut artifacts, &mut paths, artifact)?;
        }
        Ok(artifacts)
    }

    fn validate_linked_graph(&self) -> Result<(), AvroGenerationError> {
        for name in &self.linked_names {
            let dependencies = self.graph.get(name).ok_or_else(|| {
                AvroInternalError::invariant(format!(
                    "linked declaration {name} is missing from the IDL dependency graph"
                ))
            })?;
            if let Some(owned) = dependencies
                .iter()
                .find(|dependency| self.owned_names.contains(*dependency))
            {
                return Err(AvroDiagnostic::unsafe_recursion(format!(
                    "linked declaration {name} depends on owned declaration {owned}"
                ))
                .into());
            }
        }
        detect_cycle(&self.graph, &self.linked_names).map_err(Into::into)
    }

    fn render_linked_declaration(
        &self,
        schema: &NamedSchema,
    ) -> Result<Artifact, AvroGenerationError> {
        let name = schema.full_name().to_string();
        let selected = BTreeSet::from([name.clone()]);
        let dependencies = self.graph.get(&name).ok_or_else(|| {
            AvroInternalError::invariant(format!(
                "linked declaration {name} is missing from the IDL dependency graph"
            ))
        })?;
        let imports = dependencies
            .iter()
            .filter(|dependency| *dependency != &name && self.linked_names.contains(*dependency))
            .cloned()
            .collect::<BTreeSet<_>>();
        let protocol_name = self.schema_protocol_name(schema.full_name())?;
        let path = protocol_path(protocol_name);
        let properties = Properties::new();
        let content = self.render_protocol_file(ProtocolFile {
            name: protocol_name,
            properties: &properties,
            doc: None,
            selected: &selected,
            imports: &imports,
            synthetic_root: None,
            messages: &[],
        })?;
        Ok(text_artifact(path, content))
    }

    fn render_root(&self, root: &AvroRoot) -> Result<Artifact, AvroGenerationError> {
        let root_name = root.full_name().to_string();
        let protocol_name = self.schema_protocol_name(root.full_name())?;
        let path = protocol_path(protocol_name);
        let referenced = root
            .referenced_named_declarations()
            .iter()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>();
        let mut selected = referenced
            .iter()
            .filter(|name| {
                self.dependencies == Dependencies::SelfContained || self.owned_names.contains(*name)
            })
            .cloned()
            .collect::<BTreeSet<_>>();
        if self.schemas.contains_key(&root_name) {
            selected.insert(root_name.clone());
        }
        let imports = if self.dependencies == Dependencies::Linked {
            referenced
                .iter()
                .filter(|name| self.linked_names.contains(*name))
                .cloned()
                .collect()
        } else {
            BTreeSet::new()
        };
        let properties = Properties::new();
        let content = self.render_protocol_file(ProtocolFile {
            name: protocol_name,
            properties: &properties,
            doc: None,
            selected: &selected,
            imports: &imports,
            synthetic_root:
                (!matches!(root.tpe(), AvroType::Named(name) if name == root.full_name())
                    || !self.schemas.contains_key(&root_name))
                .then_some(root),
            messages: &[],
        })?;
        Ok(text_artifact(path, content))
    }

    fn render_protocol(&self, protocol: &Protocol) -> Result<Artifact, AvroGenerationError> {
        let referenced = protocol
            .referenced_named_declarations()
            .iter()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>();
        let selected = referenced
            .iter()
            .filter(|name| {
                self.dependencies == Dependencies::SelfContained || self.owned_names.contains(*name)
            })
            .cloned()
            .collect::<BTreeSet<_>>();
        let imports = if self.dependencies == Dependencies::Linked {
            referenced
                .iter()
                .filter(|name| self.linked_names.contains(*name))
                .cloned()
                .collect()
        } else {
            BTreeSet::new()
        };
        let content = self.render_protocol_file(ProtocolFile {
            name: protocol.full_name(),
            properties: protocol.properties(),
            doc: property_string(protocol.properties(), "morphir.doc"),
            selected: &selected,
            imports: &imports,
            synthetic_root: None,
            messages: protocol.messages(),
        })?;
        Ok(text_artifact(protocol_path(protocol.full_name()), content))
    }

    fn render_protocol_file(&self, file: ProtocolFile<'_>) -> Result<String, AvroGenerationError> {
        let mut output = String::new();
        if file.messages.iter().any(message_requires_javacc) {
            output.push_str(
                "// Avro Tools 1.12.2 requires `idl --useJavaCC` for message annotations with named responses.\n",
            );
        }
        render_doc(&mut output, "", file.doc);
        render_annotations(&mut output, "", file.properties)?;
        annotation(
            &mut output,
            "",
            "namespace",
            &Value::String(file.name.namespace().to_owned()),
        )?;
        output.push_str("protocol ");
        output.push_str(&escape_idl_identifier(file.name.name()));
        output.push_str(" {\n");

        let current_path = protocol_path(file.name);
        for imported in file.imports {
            let schema = self
                .schemas
                .get(imported)
                .ok_or_else(|| AvroDiagnostic::missing_linked_dependency(imported))?;
            let imported_path = protocol_path(self.schema_protocol_name(schema.full_name())?);
            output.push_str("  import idl ");
            output.push_str(&json_string(&relative_path(&current_path, &imported_path))?);
            output.push_str(";\n");
        }
        if !file.imports.is_empty()
            && (!file.selected.is_empty()
                || file.synthetic_root.is_some()
                || !file.messages.is_empty())
        {
            output.push('\n');
        }

        let declarations = self.declarations_in_dependency_order(file.selected)?;
        for (index, schema) in declarations.iter().enumerate() {
            self.render_named(&mut output, schema)?;
            if index + 1 < declarations.len()
                || file.synthetic_root.is_some()
                || !file.messages.is_empty()
            {
                output.push('\n');
            }
        }
        if let Some(root) = file.synthetic_root {
            self.render_synthetic_root(&mut output, root)?;
            if !file.messages.is_empty() {
                output.push('\n');
            }
        }
        for (index, message) in file.messages.iter().enumerate() {
            self.render_message(&mut output, message)?;
            if index + 1 < file.messages.len() {
                output.push('\n');
            }
        }
        output.push_str("}\n");
        Ok(output)
    }
}

struct ProtocolFile<'a> {
    name: &'a AvroFullName,
    properties: &'a Properties,
    doc: Option<&'a str>,
    selected: &'a BTreeSet<String>,
    imports: &'a BTreeSet<String>,
    synthetic_root: Option<&'a AvroRoot>,
    messages: &'a [AvroMessage],
}

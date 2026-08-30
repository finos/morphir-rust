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
    IdlRenderer::new(package, dependencies).render()
}

struct IdlRenderer<'package> {
    package: &'package AvroPackage,
    dependencies: Dependencies,
    schemas: BTreeMap<String, &'package NamedSchema>,
    owned_names: BTreeSet<String>,
    linked_names: BTreeSet<String>,
    graph: BTreeMap<String, BTreeSet<String>>,
}

impl<'package> IdlRenderer<'package> {
    fn new(package: &'package AvroPackage, dependencies: Dependencies) -> Self {
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
        Self {
            package,
            dependencies,
            schemas,
            owned_names,
            linked_names,
            graph,
        }
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
        let protocol_name = schema_protocol_name(schema.full_name())?;
        let path = protocol_path(&protocol_name);
        let properties = Properties::new();
        let content = self.render_protocol_file(ProtocolFile {
            name: &protocol_name,
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
        let protocol_name = schema_protocol_name(root.full_name())?;
        let path = protocol_path(&protocol_name);
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
            name: &protocol_name,
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
            let imported_path = protocol_path(&schema_protocol_name(schema.full_name())?);
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

    fn render_synthetic_root(
        &self,
        output: &mut String,
        root: &AvroRoot,
    ) -> Result<(), AvroDiagnostic> {
        render_doc(output, "  ", root.doc());
        annotation(
            output,
            "  ",
            "namespace",
            &Value::String(root.full_name().namespace().to_owned()),
        )?;
        render_annotations(output, "  ", root.properties())?;
        output.push_str("  record ");
        output.push_str(&escape_idl_identifier(root.full_name().name()));
        output.push_str(" {\n    ");
        output.push_str(&render_type(root.tpe())?);
        output.push_str(" value;\n  }\n");
        Ok(())
    }

    fn declarations_in_dependency_order(
        &self,
        selected: &BTreeSet<String>,
    ) -> Result<Vec<&'package NamedSchema>, AvroDiagnostic> {
        detect_cycle(&self.graph, selected)?;
        let mut visited = BTreeSet::new();
        let mut ordered = Vec::new();
        for name in selected {
            self.visit_declaration(name, selected, &mut visited, &mut ordered);
        }
        ordered
            .into_iter()
            .map(|name| {
                self.schemas
                    .get(&name)
                    .copied()
                    .ok_or_else(|| AvroDiagnostic::missing_linked_dependency(name))
            })
            .collect()
    }

    fn visit_declaration(
        &self,
        name: &str,
        selected: &BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        ordered: &mut Vec<String>,
    ) {
        if !visited.insert(name.to_owned()) {
            return;
        }
        if let Some(dependencies) = self.graph.get(name) {
            for dependency in dependencies {
                if dependency != name && selected.contains(dependency) {
                    self.visit_declaration(dependency, selected, visited, ordered);
                }
            }
        }
        ordered.push(name.to_owned());
    }

    fn render_named(
        &self,
        output: &mut String,
        schema: &NamedSchema,
    ) -> Result<(), AvroDiagnostic> {
        render_doc(output, "  ", schema.doc());
        annotation(
            output,
            "  ",
            "namespace",
            &Value::String(schema.full_name().namespace().to_owned()),
        )?;
        match schema {
            NamedSchema::Record(record) => {
                render_annotations(output, "  ", record.properties())?;
                output.push_str("  record ");
                output.push_str(&escape_idl_identifier(record.full_name().name()));
                output.push_str(" {\n");
                for field in record.fields() {
                    self.render_field(output, field, "    ")?;
                }
                output.push_str("  }\n");
            }
            NamedSchema::Enum(schema) => {
                render_annotations(output, "  ", schema.properties())?;
                output.push_str("  enum ");
                output.push_str(&escape_idl_identifier(schema.full_name().name()));
                output.push_str(" { ");
                output.push_str(
                    &schema
                        .symbols()
                        .iter()
                        .map(|symbol| escape_idl_identifier(symbol))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                output.push_str(" }\n");
            }
            NamedSchema::Fixed(schema) => {
                render_annotations(output, "  ", schema.properties())?;
                output.push_str("  fixed ");
                output.push_str(&escape_idl_identifier(schema.full_name().name()));
                output.push('(');
                output.push_str(&schema.size().to_string());
                output.push_str(");\n");
            }
        }
        Ok(())
    }

    fn render_field(
        &self,
        output: &mut String,
        field: &AvroField,
        indent: &str,
    ) -> Result<(), AvroDiagnostic> {
        output.push_str(indent);
        output.push_str(&render_type(field.tpe())?);
        output.push(' ');
        render_inline_annotations(output, field.properties())?;
        output.push_str(&escape_idl_identifier(field.name()));
        output.push_str(";\n");
        Ok(())
    }

    fn render_message(
        &self,
        output: &mut String,
        message: &AvroMessage,
    ) -> Result<(), AvroDiagnostic> {
        render_doc(
            output,
            "  ",
            property_string(message.properties(), "morphir.doc"),
        );
        render_annotations(output, "  ", message.properties())?;
        output.push_str("  ");
        output.push_str(&render_type(message.response())?);
        output.push(' ');
        output.push_str(&escape_idl_identifier(message.name()));
        output.push('(');
        for (index, field) in message.request().fields().iter().enumerate() {
            if index > 0 {
                output.push_str(", ");
            }
            output.push_str(&render_type(field.tpe())?);
            output.push(' ');
            render_inline_annotations(output, field.properties())?;
            output.push_str(&escape_idl_identifier(field.name()));
        }
        output.push(')');
        if !message.errors().is_empty() {
            output.push_str(" throws ");
            let errors = message
                .errors()
                .iter()
                .map(render_type)
                .collect::<Result<Vec<_>, _>>()?;
            output.push_str(&errors.join(", "));
        }
        output.push_str(";\n");
        Ok(())
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

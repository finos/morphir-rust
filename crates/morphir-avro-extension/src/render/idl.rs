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

        if self.package.protocols().is_empty() {
            for root in self.package.roots() {
                let artifact = self.render_root(root)?;
                insert_artifact(&mut artifacts, &mut paths, artifact)?;
            }
        } else {
            for protocol in self.package.protocols() {
                let artifact = self.render_protocol(protocol)?;
                insert_artifact(&mut artifacts, &mut paths, artifact)?;
            }
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

fn schema_protocol_name(name: &AvroFullName) -> Result<AvroFullName, AvroInternalError> {
    AvroFullName::new(
        name.namespace().to_owned(),
        format!("{}Schemas", name.name()),
    )
    .map_err(|error| {
        AvroInternalError::invariant(format!(
            "schema protocol suffix invalidated checked Avro name {name}: {error}"
        ))
    })
}

fn collect_schema_references(schema: &NamedSchema, references: &mut BTreeSet<String>) {
    if let NamedSchema::Record(record) = schema {
        for field in record.fields() {
            collect_type_references(field.tpe(), references);
        }
    }
}

fn collect_type_references(tpe: &AvroType, references: &mut BTreeSet<String>) {
    match tpe {
        AvroType::Named(name) => {
            references.insert(name.to_string());
        }
        AvroType::Array(items, _) | AvroType::Map(items, _) => {
            collect_type_references(items, references);
        }
        AvroType::Union(union) => {
            for branch in union.branches() {
                collect_type_references(branch, references);
            }
        }
        AvroType::Logical { physical, .. } | AvroType::Annotated { physical, .. } => {
            collect_type_references(physical, references);
        }
        AvroType::Null
        | AvroType::Boolean
        | AvroType::Int
        | AvroType::Long
        | AvroType::Float
        | AvroType::Double
        | AvroType::Bytes
        | AvroType::String => {}
    }
}

fn render_type(tpe: &AvroType) -> Result<String, AvroDiagnostic> {
    Ok(match tpe {
        AvroType::Null => "null".to_owned(),
        AvroType::Boolean => "boolean".to_owned(),
        AvroType::Int => "int".to_owned(),
        AvroType::Long => "long".to_owned(),
        AvroType::Float => "float".to_owned(),
        AvroType::Double => "double".to_owned(),
        AvroType::Bytes => "bytes".to_owned(),
        AvroType::String => "string".to_owned(),
        AvroType::Array(items, properties) => {
            format!(
                "{}array<{}>",
                inline_annotations(properties)?,
                render_type(items)?
            )
        }
        AvroType::Map(values, properties) => {
            format!(
                "{}map<{}>",
                inline_annotations(properties)?,
                render_type(values)?
            )
        }
        AvroType::Union(union) => format!(
            "union {{ {} }}",
            union
                .branches()
                .iter()
                .map(render_type)
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        ),
        AvroType::Named(name) => render_full_name(name),
        AvroType::Logical {
            physical,
            name,
            properties,
        } => render_logical_type(physical, name, properties)?,
        AvroType::Annotated {
            physical,
            properties,
        } => format!(
            "{}{}",
            inline_annotations(properties)?,
            render_type(physical)?
        ),
    })
}

fn render_logical_type(
    physical: &AvroType,
    name: &str,
    properties: &Properties,
) -> Result<String, AvroDiagnostic> {
    let shorthand = match (name, physical) {
        ("decimal", AvroType::Bytes) => {
            let precision = property_integer(properties, "precision")?;
            let scale = property_integer(properties, "scale")?;
            Some((
                format!("decimal({precision}, {scale})"),
                &["precision", "scale"][..],
            ))
        }
        ("date", AvroType::Int) => Some(("date".to_owned(), &[][..])),
        ("time-millis", AvroType::Long) => Some(("time_ms".to_owned(), &[][..])),
        ("timestamp-millis", AvroType::Long) => Some(("timestamp_ms".to_owned(), &[][..])),
        ("local-timestamp-millis", AvroType::Long) => {
            Some(("local_timestamp_ms".to_owned(), &[][..]))
        }
        ("uuid", AvroType::String) => Some(("uuid".to_owned(), &[][..])),
        _ => None,
    };

    if let Some((shorthand, encoded_properties)) = shorthand {
        let annotations = properties
            .iter()
            .filter(|(key, _)| {
                key.as_str() != "logicalType" && !encoded_properties.contains(&key.as_str())
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Properties>();
        return Ok(format!(
            "{}{}",
            inline_annotations(&annotations)?,
            shorthand
        ));
    }

    let mut annotations = properties.clone();
    annotations.insert("logicalType".to_owned(), Value::String(name.to_owned()));
    Ok(format!(
        "{}{}",
        inline_annotations(&annotations)?,
        render_type(physical)?
    ))
}

fn message_requires_javacc(message: &AvroMessage) -> bool {
    !message.properties().is_empty() && type_contains_named(message.response())
}

fn type_contains_named(tpe: &AvroType) -> bool {
    match tpe {
        AvroType::Named(_) => true,
        AvroType::Array(items, _) | AvroType::Map(items, _) => type_contains_named(items),
        AvroType::Union(union) => union.branches().iter().any(type_contains_named),
        AvroType::Logical { physical, .. } | AvroType::Annotated { physical, .. } => {
            type_contains_named(physical)
        }
        AvroType::Null
        | AvroType::Boolean
        | AvroType::Int
        | AvroType::Long
        | AvroType::Float
        | AvroType::Double
        | AvroType::Bytes
        | AvroType::String => false,
    }
}

fn property_integer(properties: &Properties, key: &str) -> Result<u64, AvroDiagnostic> {
    properties
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| AvroDiagnostic::invalid_option(format!("logical type property {key}")))
}

fn render_full_name(name: &AvroFullName) -> String {
    name.namespace()
        .split('.')
        .chain(std::iter::once(name.name()))
        .map(escape_idl_identifier)
        .collect::<Vec<_>>()
        .join(".")
}

fn render_annotations(
    output: &mut String,
    indent: &str,
    properties: &Properties,
) -> Result<(), AvroDiagnostic> {
    for (name, value) in properties {
        annotation(output, indent, name, value)?;
    }
    Ok(())
}

fn annotation(
    output: &mut String,
    indent: &str,
    name: &str,
    value: &Value,
) -> Result<(), AvroDiagnostic> {
    output.push_str(indent);
    output.push('@');
    output.push_str(name);
    output.push('(');
    output.push_str(&json_value(value)?);
    output.push_str(")\n");
    Ok(())
}

fn render_inline_annotations(
    output: &mut String,
    properties: &Properties,
) -> Result<(), AvroDiagnostic> {
    output.push_str(&inline_annotations(properties)?);
    Ok(())
}

fn inline_annotations(properties: &Properties) -> Result<String, AvroDiagnostic> {
    let mut output = String::new();
    for (name, value) in properties {
        output.push('@');
        output.push_str(name);
        output.push('(');
        output.push_str(&json_value(value)?);
        output.push_str(") ");
    }
    Ok(output)
}

fn property_string<'properties>(
    properties: &'properties Properties,
    key: &str,
) -> Option<&'properties str> {
    properties.get(key).and_then(Value::as_str)
}

fn render_doc(output: &mut String, indent: &str, doc: Option<&str>) {
    let Some(doc) = doc else {
        return;
    };
    output.push_str(indent);
    output.push_str("/**\n");
    for line in doc.lines() {
        output.push_str(indent);
        output.push_str(" * ");
        output.push_str(&escape_doc_line(line));
        output.push('\n');
    }
    output.push_str(indent);
    output.push_str(" */\n");
}

fn escape_doc_line(line: &str) -> String {
    line.chars()
        .map(|character| {
            if character.is_control() {
                format!("\\u{:04x}", u32::from(character))
            } else {
                character.to_string()
            }
        })
        .collect::<String>()
        .replace("*/", "* /")
}

fn json_value(value: &Value) -> Result<String, AvroDiagnostic> {
    serde_json::to_string(value)
        .map_err(|error| AvroDiagnostic::invalid_option(format!("render IDL annotation: {error}")))
}

fn json_string(value: &str) -> Result<String, AvroDiagnostic> {
    json_value(&Value::String(value.to_owned()))
}

fn protocol_path(name: &AvroFullName) -> String {
    let mut components = name.namespace().split('.').collect::<Vec<_>>();
    components.retain(|component| !component.is_empty());
    components.push(name.name());
    format!("{}.avdl", components.join("/"))
}

fn relative_path(from: &str, to: &str) -> String {
    let mut from = from.split('/').collect::<Vec<_>>();
    from.pop();
    let to = to.split('/').collect::<Vec<_>>();
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    std::iter::repeat_n("..", from.len() - common)
        .chain(to[common..].iter().copied())
        .collect::<Vec<_>>()
        .join("/")
}

fn detect_cycle(
    graph: &BTreeMap<String, BTreeSet<String>>,
    selected: &BTreeSet<String>,
) -> Result<(), AvroDiagnostic> {
    fn visit(
        name: &str,
        graph: &BTreeMap<String, BTreeSet<String>>,
        selected: &BTreeSet<String>,
        visiting: &mut Vec<String>,
        visited: &mut BTreeSet<String>,
    ) -> Result<(), AvroDiagnostic> {
        if visited.contains(name) {
            return Ok(());
        }
        if let Some(start) = visiting.iter().position(|active| active == name) {
            let cycle = &visiting[start..];
            if cycle.len() > 1 {
                return Err(AvroDiagnostic::unsafe_recursion(cycle.join(" -> ")));
            }
            return Ok(());
        }
        visiting.push(name.to_owned());
        if let Some(dependencies) = graph.get(name) {
            for dependency in dependencies {
                if selected.contains(dependency) && dependency != name {
                    visit(dependency, graph, selected, visiting, visited)?;
                }
            }
        }
        visiting.pop();
        visited.insert(name.to_owned());
        Ok(())
    }

    let mut visiting = Vec::new();
    let mut visited = BTreeSet::new();
    for name in selected {
        visit(name, graph, selected, &mut visiting, &mut visited)?;
    }
    Ok(())
}

fn text_artifact(path: String, content: String) -> Artifact {
    Artifact {
        path,
        content,
        binary: false,
    }
}

fn insert_artifact(
    artifacts: &mut Vec<Artifact>,
    paths: &mut BTreeSet<String>,
    artifact: Artifact,
) -> Result<(), AvroDiagnostic> {
    if !paths.insert(artifact.path.clone()) {
        return Err(AvroDiagnostic::name_collision(format!(
            "duplicate artifact path {}",
            artifact.path
        )));
    }
    artifacts.push(artifact);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AvroRequest, RecordSchema};

    #[test]
    fn missing_linked_graph_node_is_an_internal_error_during_validation() {
        let package =
            AvroPackage::new(Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()).unwrap();
        let mut renderer = IdlRenderer::new(&package, Dependencies::Linked);
        renderer.linked_names.insert("example.Missing".to_owned());

        assert!(matches!(
            renderer.validate_linked_graph(),
            Err(AvroGenerationError::Internal(_))
        ));
    }

    #[test]
    fn missing_linked_graph_node_is_an_internal_error_during_rendering() {
        let package =
            AvroPackage::new(Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()).unwrap();
        let renderer = IdlRenderer::new(&package, Dependencies::Linked);
        let schema = NamedSchema::Record(
            RecordSchema::new(
                AvroFullName::new("example".to_owned(), "Missing".to_owned()).unwrap(),
                Vec::new(),
                None,
                Properties::new(),
            )
            .unwrap(),
        );

        assert!(matches!(
            renderer.render_linked_declaration(&schema),
            Err(AvroGenerationError::Internal(_))
        ));
    }

    #[test]
    fn identifiers_are_case_sensitive_in_protocol_type_field_and_message_contexts() {
        let cases = [
            (
                ["protocol", "record", "string", "error"],
                ["`protocol`", "`record`", "`string`", "`error`"],
            ),
            (
                [
                    "time_micros",
                    "timestamp_micros",
                    "local_timestamp_micros",
                    "BigDecimal",
                ],
                [
                    "time_micros",
                    "timestamp_micros",
                    "local_timestamp_micros",
                    "BigDecimal",
                ],
            ),
        ];
        for (identifiers, expected) in cases {
            let content = render_identifier_contexts(identifiers);
            let contexts = [
                ("protocol", format!("protocol {}", expected[0])),
                ("type", format!("record {}", expected[1])),
                ("field", format!("string {};", expected[2])),
                (
                    "message",
                    format!("example.{} {}();", expected[1], expected[3]),
                ),
            ];
            for (context, expected) in contexts {
                assert!(
                    content.contains(&expected),
                    "missing {context} context {expected:?} in {content}"
                );
            }
        }
    }

    fn render_identifier_contexts(identifiers: [&str; 4]) -> String {
        let [
            protocol_identifier,
            type_identifier,
            field_identifier,
            message_identifier,
        ] = identifiers;
        let record_name =
            AvroFullName::new("example".to_owned(), type_identifier.to_owned()).unwrap();
        let record = NamedSchema::Record(
            RecordSchema::new(
                record_name.clone(),
                vec![
                    AvroField::new(
                        field_identifier.to_owned(),
                        AvroType::String,
                        Properties::new(),
                    )
                    .unwrap(),
                ],
                None,
                Properties::new(),
            )
            .unwrap(),
        );
        let message = AvroMessage::new(
            message_identifier.to_owned(),
            AvroRequest::new(Vec::new()).unwrap(),
            AvroType::Named(record_name.clone()),
            Vec::new(),
            Properties::from([(
                "morphir.value-kind".to_owned(),
                Value::String("function".to_owned()),
            )]),
        )
        .unwrap();
        let protocol = Protocol::new(
            AvroFullName::new("example".to_owned(), protocol_identifier.to_owned()).unwrap(),
            vec![message],
            vec![AvroType::Named(record_name)],
            Properties::new(),
        )
        .unwrap();
        let package = AvroPackage::new(
            Vec::new(),
            vec![record],
            Vec::new(),
            vec![protocol],
            Vec::new(),
        )
        .unwrap();

        let artifacts = render_idl(&package, Dependencies::SelfContained).unwrap();
        assert_eq!(artifacts.len(), 1);
        artifacts.into_iter().next().unwrap().content
    }

    #[test]
    fn logical_shorthand_requires_the_canonical_physical_type_and_keeps_custom_properties() {
        let cases = [
            (
                AvroType::Int,
                "date",
                Properties::from([(
                    "morphir.fqname".to_owned(),
                    Value::String("example:types#date".to_owned()),
                )]),
                "@morphir.fqname(\"example:types#date\") date",
            ),
            (
                AvroType::String,
                "uuid",
                Properties::from([(
                    "morphir.fqname".to_owned(),
                    Value::String("example:types#identifier".to_owned()),
                )]),
                "@morphir.fqname(\"example:types#identifier\") uuid",
            ),
            (
                AvroType::Bytes,
                "decimal",
                Properties::from([
                    (
                        "morphir.fqname".to_owned(),
                        Value::String("example:types#amount".to_owned()),
                    ),
                    ("precision".to_owned(), Value::from(20)),
                    ("scale".to_owned(), Value::from(4)),
                ]),
                "@morphir.fqname(\"example:types#amount\") decimal(20, 4)",
            ),
        ];

        for (physical, logical, properties, expected) in cases {
            let actual = render_type(&AvroType::Logical {
                physical: Box::new(physical),
                name: logical.to_owned(),
                properties,
            })
            .unwrap();
            assert_eq!(actual, expected, "logical type {logical}");
        }
    }

    #[test]
    fn noncanonical_logical_mappings_keep_the_configured_physical_type() {
        let cases = [
            (AvroType::Long, "date", "long"),
            (AvroType::Bytes, "uuid", "bytes"),
            (AvroType::String, "decimal", "string"),
        ];

        for (physical, logical, expected_physical) in cases {
            let actual = render_type(&AvroType::Logical {
                physical: Box::new(physical),
                name: logical.to_owned(),
                properties: Properties::from([(
                    "morphir.fqname".to_owned(),
                    Value::String(format!("example:types#{logical}")),
                )]),
            })
            .unwrap();
            assert_eq!(
                actual,
                format!(
                    "@logicalType(\"{logical}\") @morphir.fqname(\"example:types#{logical}\") {expected_physical}"
                )
            );
        }
    }

    #[test]
    fn affected_protocols_lead_with_the_javacc_compatibility_notice() {
        let content = render_identifier_contexts(["example", "response", "input", "find"]);
        assert!(content.starts_with(
            "// Avro Tools 1.12.2 requires `idl --useJavaCC` for message annotations with named responses.\n"
        ));
    }

    #[test]
    fn documentation_and_annotation_strings_escape_idl_terminators_and_controls() {
        let mut doc = String::new();
        render_doc(
            &mut doc,
            "  ",
            Some("first */ line\\path\nsecond\u{0001}line"),
        );
        assert_eq!(
            doc,
            "  /**\n   * first * / line\\path\n   * second\\u0001line\n   */\n"
        );
        assert_eq!(
            json_value(&Value::String("line\\path\ncontrol\u{0001}".to_owned())).unwrap(),
            "\"line\\\\path\\ncontrol\\u0001\""
        );
    }
}

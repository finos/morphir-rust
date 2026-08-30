use std::collections::{BTreeMap, BTreeSet};

use morphir_extension_sdk::Artifact;
use serde_json::{Map, Number, Value};

use crate::{
    AvroDiagnostic, AvroField, AvroFullName, AvroGenerationError, AvroInternalError, AvroPackage,
    AvroRoot, AvroType, Dependencies, NamedSchema, Properties, Protocol,
};

/// Render a checked Avro package as deterministic Avro JSON artifacts.
///
/// Every schema root gets its own `.avsc` artifact. Protocol projection emits
/// one `.avpr` artifact per module. Linked declarations are emitted once as
/// separate `.avsc` artifacts.
pub fn render_json(
    package: &AvroPackage,
    dependencies: Dependencies,
) -> Result<Vec<Artifact>, AvroGenerationError> {
    let renderer = JsonRenderer::new(package, dependencies);
    renderer.render()
}

struct JsonRenderer<'package> {
    package: &'package AvroPackage,
    dependencies: Dependencies,
    schemas: BTreeMap<String, &'package NamedSchema>,
    linked_names: BTreeSet<String>,
}

impl<'package> JsonRenderer<'package> {
    fn new(package: &'package AvroPackage, dependencies: Dependencies) -> Self {
        let schemas = package
            .schemas()
            .iter()
            .chain(package.linked_schemas())
            .map(|schema| (schema.full_name().to_string(), schema))
            .collect();
        let linked_names = package
            .linked_schemas()
            .iter()
            .map(|schema| schema.full_name().to_string())
            .collect();
        Self {
            package,
            dependencies,
            schemas,
            linked_names,
        }
    }

    fn render(self) -> Result<Vec<Artifact>, AvroGenerationError> {
        let mut artifacts = Vec::new();
        let mut paths = BTreeSet::new();
        let mut predefined = BTreeSet::new();
        if self.dependencies == Dependencies::Linked {
            for (name, value, defined_names) in self.linked_registry_definitions()? {
                let schema = self.schemas.get(&name).ok_or_else(|| {
                    AvroInternalError::invariant(format!(
                        "JSON registry leader {name} has no schema"
                    ))
                })?;
                let artifact = text_artifact(schema_path(schema.full_name()), value)?;
                insert_artifact(&mut artifacts, &mut paths, artifact)?;
                predefined.extend(defined_names);
            }
        }
        if self.package.protocols().is_empty() {
            for root in self.package.roots() {
                if paths.contains(&schema_path(root.full_name())) {
                    continue;
                }
                let artifact = self.render_root(root, &predefined)?;
                insert_artifact(&mut artifacts, &mut paths, artifact)?;
            }
        }
        for protocol in self.package.protocols() {
            let artifact = self.render_protocol(protocol, &predefined)?;
            insert_artifact(&mut artifacts, &mut paths, artifact)?;
        }
        Ok(artifacts)
    }

    fn linked_registry_definitions(
        &self,
    ) -> Result<Vec<(String, Value, BTreeSet<String>)>, AvroInternalError> {
        let graph = self.schema_graph();
        let components = strongly_connected_components(&graph)?;
        let component_by_name = components
            .iter()
            .enumerate()
            .flat_map(|(index, component)| component.iter().cloned().map(move |name| (name, index)))
            .collect::<BTreeMap<_, _>>();
        let component_dependencies = components
            .iter()
            .enumerate()
            .map(|(component_index, component)| {
                component
                    .iter()
                    .flat_map(|name| graph.get(name).into_iter().flatten())
                    .filter_map(|dependency| component_by_name.get(dependency).copied())
                    .filter(|dependency| *dependency != component_index)
                    .collect::<BTreeSet<_>>()
            })
            .collect::<Vec<_>>();
        let mut early = components
            .iter()
            .enumerate()
            .filter(|(_, component)| {
                component.len() > 1
                    || component
                        .iter()
                        .any(|name| self.linked_names.contains(name))
            })
            .map(|(index, _)| index)
            .collect::<BTreeSet<_>>();
        let seeds = early.iter().copied().collect::<Vec<_>>();
        for seed in seeds {
            include_component_dependencies(seed, &component_dependencies, &mut early)?;
        }

        let mut definitions = Vec::new();
        for index in component_dependency_order(&components, &component_dependencies)? {
            if !early.contains(&index) {
                continue;
            }
            let component = components.get(index).ok_or_else(|| {
                AvroInternalError::invariant(format!(
                    "JSON schema component index {index} is missing"
                ))
            })?;
            let members = component.iter().cloned().collect::<BTreeSet<_>>();
            let leader = component.first().ok_or_else(|| {
                AvroInternalError::invariant("JSON schema graph produced an empty component")
            })?;
            let schema = self.schemas.get(leader).ok_or_else(|| {
                AvroInternalError::invariant(format!(
                    "JSON schema component leader {leader} is missing"
                ))
            })?;
            let value = if members.len() > 1 {
                let mut state = InlineState::default();
                self.render_named_inline(
                    schema.full_name(),
                    &mut state,
                    DefinitionScope::Selected(&members),
                )
            } else {
                self.render_named_reference_only(schema)
            };
            definitions.push((leader.clone(), value, members));
        }
        Ok(definitions)
    }

    fn schema_graph(&self) -> BTreeMap<String, BTreeSet<String>> {
        self.schemas
            .iter()
            .map(|(name, schema)| {
                let mut dependencies = BTreeSet::new();
                if let NamedSchema::Record(record) = schema {
                    for field in record.fields() {
                        collect_available_references(field.tpe(), &self.schemas, &mut dependencies);
                    }
                }
                (name.clone(), dependencies)
            })
            .collect()
    }

    fn render_root(
        &self,
        root: &AvroRoot,
        predefined: &BTreeSet<String>,
    ) -> Result<Artifact, AvroDiagnostic> {
        let external_named_target = matches!(
            root.tpe(),
            AvroType::Named(name)
                if self.dependencies == Dependencies::Linked
                    && predefined.contains(&name.to_string())
        );
        let nested_named_target = matches!(
            root.tpe(),
            AvroType::Named(name)
                if name != root.full_name() && !predefined.contains(&name.to_string())
        );
        let value = if self.dependencies == Dependencies::SelfContained {
            let mut state = InlineState::default();
            match root.tpe() {
                AvroType::Named(name) if self.schemas.contains_key(&name.to_string()) => {
                    self.render_named_inline(name, &mut state, DefinitionScope::All)
                }
                tpe => self.render_type_inline(tpe, &mut state, DefinitionScope::All),
            }
        } else {
            let mut state = InlineState {
                defined: predefined.clone(),
                ..InlineState::default()
            };
            match root.tpe() {
                AvroType::Named(name) => {
                    self.render_named_inline(name, &mut state, DefinitionScope::Owned)
                }
                tpe => self.render_type_inline(tpe, &mut state, DefinitionScope::Owned),
            }
        };
        let value = if external_named_target {
            value
        } else {
            decorate_root(value, root, nested_named_target)
        };
        text_artifact(schema_path(root.full_name()), value)
    }

    fn render_protocol(
        &self,
        protocol: &Protocol,
        predefined: &BTreeSet<String>,
    ) -> Result<Artifact, AvroDiagnostic> {
        let mut object = properties_object(protocol.properties());
        object.insert(
            "messages".to_owned(),
            Value::Object(
                protocol
                    .messages()
                    .iter()
                    .map(|message| {
                        let mut rendered = properties_object(message.properties());
                        rendered.insert(
                            "errors".to_owned(),
                            Value::Array(
                                message
                                    .errors()
                                    .iter()
                                    .map(|error| self.render_type_reference_only(error))
                                    .collect(),
                            ),
                        );
                        rendered.insert(
                            "request".to_owned(),
                            Value::Array(
                                message
                                    .request()
                                    .fields()
                                    .iter()
                                    .map(|field| self.render_field_reference_only(field))
                                    .collect(),
                            ),
                        );
                        rendered.insert(
                            "response".to_owned(),
                            self.render_type_reference_only(message.response()),
                        );
                        (message.name().to_owned(), Value::Object(rendered))
                    })
                    .collect(),
            ),
        );
        object.insert(
            "namespace".to_owned(),
            Value::String(protocol.full_name().namespace().to_owned()),
        );
        object.insert(
            "protocol".to_owned(),
            Value::String(protocol.full_name().name().to_owned()),
        );
        let scope = if self.dependencies == Dependencies::SelfContained {
            DefinitionScope::All
        } else {
            DefinitionScope::Owned
        };
        let mut state = InlineState {
            defined: predefined.clone(),
            ..InlineState::default()
        };
        let mut types = Vec::new();
        for name in protocol.referenced_named_declarations() {
            let key = name.to_string();
            if state.defined.contains(&key) || !self.schemas.contains_key(&key) {
                continue;
            }
            types.push(self.render_named_inline(name, &mut state, scope));
        }
        object.insert("types".to_owned(), Value::Array(types));
        text_artifact(protocol_path(protocol.full_name()), Value::Object(object))
    }

    fn render_named_inline(
        &self,
        name: &AvroFullName,
        state: &mut InlineState,
        scope: DefinitionScope<'_>,
    ) -> Value {
        let key = name.to_string();
        if state.active.contains(&key)
            || state.defined.contains(&key)
            || !scope.allows(&key, &self.linked_names)
        {
            return Value::String(key);
        }
        let Some(schema) = self.schemas.get(&key) else {
            return Value::String(key);
        };
        state.active.insert(key.clone());
        state.defined.insert(key.clone());
        let value = self.render_named(schema, |tpe| self.render_type_inline(tpe, state, scope));
        state.active.remove(&key);
        value
    }

    fn render_named_reference_only(&self, schema: &NamedSchema) -> Value {
        self.render_named(schema, |tpe| self.render_type_reference_only(tpe))
    }

    fn render_named(
        &self,
        schema: &NamedSchema,
        mut render_type: impl FnMut(&AvroType) -> Value,
    ) -> Value {
        match schema {
            NamedSchema::Record(record) => {
                let mut object = properties_object(record.properties());
                insert_doc(&mut object, record.doc());
                object.insert(
                    "fields".to_owned(),
                    Value::Array(
                        record
                            .fields()
                            .iter()
                            .map(|field| render_field(field, &mut render_type))
                            .collect(),
                    ),
                );
                insert_name(&mut object, record.full_name());
                object.insert("type".to_owned(), Value::String("record".to_owned()));
                Value::Object(object)
            }
            NamedSchema::Enum(schema) => {
                let mut object = properties_object(schema.properties());
                insert_doc(&mut object, schema.doc());
                insert_name(&mut object, schema.full_name());
                object.insert(
                    "symbols".to_owned(),
                    Value::Array(
                        schema
                            .symbols()
                            .iter()
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                );
                object.insert("type".to_owned(), Value::String("enum".to_owned()));
                Value::Object(object)
            }
            NamedSchema::Fixed(schema) => {
                let mut object = properties_object(schema.properties());
                insert_doc(&mut object, schema.doc());
                insert_name(&mut object, schema.full_name());
                object.insert(
                    "size".to_owned(),
                    Value::Number(Number::from(schema.size())),
                );
                object.insert("type".to_owned(), Value::String("fixed".to_owned()));
                Value::Object(object)
            }
        }
    }

    fn render_field_reference_only(&self, field: &AvroField) -> Value {
        render_field(field, &mut |tpe| self.render_type_reference_only(tpe))
    }

    fn render_type_inline(
        &self,
        tpe: &AvroType,
        state: &mut InlineState,
        scope: DefinitionScope<'_>,
    ) -> Value {
        match tpe {
            AvroType::Named(name) => self.render_named_inline(name, state, scope),
            AvroType::Array(items, properties) => {
                let mut object = properties_object(properties);
                object.insert(
                    "items".to_owned(),
                    self.render_type_inline(items, state, scope),
                );
                object.insert("type".to_owned(), Value::String("array".to_owned()));
                Value::Object(object)
            }
            AvroType::Map(values, properties) => {
                let mut object = properties_object(properties);
                object.insert("type".to_owned(), Value::String("map".to_owned()));
                object.insert(
                    "values".to_owned(),
                    self.render_type_inline(values, state, scope),
                );
                Value::Object(object)
            }
            AvroType::Union(union) => Value::Array(
                union
                    .branches()
                    .iter()
                    .map(|branch| self.render_type_inline(branch, state, scope))
                    .collect(),
            ),
            AvroType::Logical {
                physical,
                name,
                properties,
            } => decorate_type(
                self.render_type_inline(physical, state, scope),
                properties,
                Some(name),
            ),
            AvroType::Annotated {
                physical,
                properties,
            } => decorate_type(
                self.render_type_inline(physical, state, scope),
                properties,
                None,
            ),
            AvroType::Null => Value::String("null".to_owned()),
            AvroType::Boolean => Value::String("boolean".to_owned()),
            AvroType::Int => Value::String("int".to_owned()),
            AvroType::Long => Value::String("long".to_owned()),
            AvroType::Float => Value::String("float".to_owned()),
            AvroType::Double => Value::String("double".to_owned()),
            AvroType::Bytes => Value::String("bytes".to_owned()),
            AvroType::String => Value::String("string".to_owned()),
        }
    }

    fn render_type_reference_only(&self, tpe: &AvroType) -> Value {
        match tpe {
            AvroType::Named(name) => Value::String(name.to_string()),
            AvroType::Array(items, properties) => {
                let mut object = properties_object(properties);
                object.insert("items".to_owned(), self.render_type_reference_only(items));
                object.insert("type".to_owned(), Value::String("array".to_owned()));
                Value::Object(object)
            }
            AvroType::Map(values, properties) => {
                let mut object = properties_object(properties);
                object.insert("type".to_owned(), Value::String("map".to_owned()));
                object.insert("values".to_owned(), self.render_type_reference_only(values));
                Value::Object(object)
            }
            AvroType::Union(union) => Value::Array(
                union
                    .branches()
                    .iter()
                    .map(|branch| self.render_type_reference_only(branch))
                    .collect(),
            ),
            AvroType::Logical {
                physical,
                name,
                properties,
            } => decorate_type(
                self.render_type_reference_only(physical),
                properties,
                Some(name),
            ),
            AvroType::Annotated {
                physical,
                properties,
            } => decorate_type(self.render_type_reference_only(physical), properties, None),
            AvroType::Null => Value::String("null".to_owned()),
            AvroType::Boolean => Value::String("boolean".to_owned()),
            AvroType::Int => Value::String("int".to_owned()),
            AvroType::Long => Value::String("long".to_owned()),
            AvroType::Float => Value::String("float".to_owned()),
            AvroType::Double => Value::String("double".to_owned()),
            AvroType::Bytes => Value::String("bytes".to_owned()),
            AvroType::String => Value::String("string".to_owned()),
        }
    }
}

#[derive(Default)]
struct InlineState {
    active: BTreeSet<String>,
    defined: BTreeSet<String>,
}

#[derive(Clone, Copy)]
enum DefinitionScope<'names> {
    All,
    Owned,
    Selected(&'names BTreeSet<String>),
}

impl DefinitionScope<'_> {
    fn allows(self, name: &str, linked_names: &BTreeSet<String>) -> bool {
        match self {
            Self::All => true,
            Self::Owned => !linked_names.contains(name),
            Self::Selected(names) => names.contains(name),
        }
    }
}

fn collect_available_references(
    tpe: &AvroType,
    schemas: &BTreeMap<String, &NamedSchema>,
    references: &mut BTreeSet<String>,
) {
    match tpe {
        AvroType::Named(name) if schemas.contains_key(&name.to_string()) => {
            references.insert(name.to_string());
        }
        AvroType::Array(element, _)
        | AvroType::Map(element, _)
        | AvroType::Logical {
            physical: element, ..
        }
        | AvroType::Annotated {
            physical: element, ..
        } => collect_available_references(element, schemas, references),
        AvroType::Union(union) => {
            for branch in union.branches() {
                collect_available_references(branch, schemas, references);
            }
        }
        AvroType::Null
        | AvroType::Boolean
        | AvroType::Int
        | AvroType::Long
        | AvroType::Float
        | AvroType::Double
        | AvroType::Bytes
        | AvroType::String
        | AvroType::Named(_) => {}
    }
}

fn strongly_connected_components(
    graph: &BTreeMap<String, BTreeSet<String>>,
) -> Result<Vec<Vec<String>>, AvroInternalError> {
    let mut tarjan = Tarjan::new(graph);
    for name in graph.keys() {
        if !tarjan.indices.contains_key(name) {
            tarjan.visit(name)?;
        }
    }
    Ok(tarjan.components)
}

struct Tarjan<'graph> {
    graph: &'graph BTreeMap<String, BTreeSet<String>>,
    next_index: usize,
    indices: BTreeMap<String, usize>,
    low_links: BTreeMap<String, usize>,
    stack: Vec<String>,
    on_stack: BTreeSet<String>,
    components: Vec<Vec<String>>,
}

impl<'graph> Tarjan<'graph> {
    fn new(graph: &'graph BTreeMap<String, BTreeSet<String>>) -> Self {
        Self {
            graph,
            next_index: 0,
            indices: BTreeMap::new(),
            low_links: BTreeMap::new(),
            stack: Vec::new(),
            on_stack: BTreeSet::new(),
            components: Vec::new(),
        }
    }

    fn visit(&mut self, name: &str) -> Result<(), AvroInternalError> {
        let index = self.next_index;
        self.next_index += 1;
        self.indices.insert(name.to_owned(), index);
        self.low_links.insert(name.to_owned(), index);
        self.stack.push(name.to_owned());
        self.on_stack.insert(name.to_owned());

        let dependencies = self.graph.get(name).cloned().ok_or_else(|| {
            AvroInternalError::invariant(format!("JSON schema graph lost node {name}"))
        })?;
        for dependency in &dependencies {
            if !self.indices.contains_key(dependency) {
                self.visit(dependency)?;
                let dependency_low = *self.low_links.get(dependency).ok_or_else(|| {
                    AvroInternalError::invariant(format!(
                        "Tarjan traversal has no low link for {dependency}"
                    ))
                })?;
                self.low_links
                    .entry(name.to_owned())
                    .and_modify(|low| *low = (*low).min(dependency_low));
            } else if self.on_stack.contains(dependency) {
                let dependency_index = *self.indices.get(dependency).ok_or_else(|| {
                    AvroInternalError::invariant(format!(
                        "Tarjan traversal has no index for {dependency}"
                    ))
                })?;
                self.low_links
                    .entry(name.to_owned())
                    .and_modify(|low| *low = (*low).min(dependency_index));
            }
        }

        let low_link = self.low_links.get(name).ok_or_else(|| {
            AvroInternalError::invariant(format!("Tarjan traversal lost low link for {name}"))
        })?;
        let index = self.indices.get(name).ok_or_else(|| {
            AvroInternalError::invariant(format!("Tarjan traversal lost index for {name}"))
        })?;
        if low_link != index {
            return Ok(());
        }
        let mut component = Vec::new();
        loop {
            let member = self.stack.pop().ok_or_else(|| {
                AvroInternalError::invariant(format!(
                    "Tarjan stack ended before component root {name}"
                ))
            })?;
            self.on_stack.remove(&member);
            let complete = member == name;
            component.push(member);
            if complete {
                break;
            }
        }
        component.sort();
        self.components.push(component);
        Ok(())
    }
}

fn include_component_dependencies(
    component: usize,
    dependencies: &[BTreeSet<usize>],
    included: &mut BTreeSet<usize>,
) -> Result<(), AvroInternalError> {
    let direct = dependencies.get(component).ok_or_else(|| {
        AvroInternalError::invariant(format!(
            "JSON component dependency index {component} is missing"
        ))
    })?;
    for dependency in direct {
        if included.insert(*dependency) {
            include_component_dependencies(*dependency, dependencies, included)?;
        }
    }
    Ok(())
}

fn component_dependency_order(
    components: &[Vec<String>],
    dependencies: &[BTreeSet<usize>],
) -> Result<Vec<usize>, AvroInternalError> {
    fn visit(
        component: usize,
        components: &[Vec<String>],
        dependencies: &[BTreeSet<usize>],
        visited: &mut BTreeSet<usize>,
        ordered: &mut Vec<usize>,
    ) -> Result<(), AvroInternalError> {
        if !visited.insert(component) {
            return Ok(());
        }
        let direct = dependencies.get(component).ok_or_else(|| {
            AvroInternalError::invariant(format!(
                "JSON component ordering index {component} is missing"
            ))
        })?;
        let mut component_dependencies = direct.iter().copied().collect::<Vec<_>>();
        component_dependencies.sort_by_key(|index| {
            components
                .get(*index)
                .and_then(|component| component.first())
                .cloned()
                .unwrap_or_default()
        });
        for dependency in component_dependencies {
            visit(dependency, components, dependencies, visited, ordered)?;
        }
        ordered.push(component);
        Ok(())
    }

    let mut seeds = (0..components.len()).collect::<Vec<_>>();
    seeds.sort_by_key(|index| {
        components
            .get(*index)
            .and_then(|component| component.first())
            .cloned()
            .unwrap_or_default()
    });
    let mut visited = BTreeSet::new();
    let mut ordered = Vec::new();
    for seed in seeds {
        visit(seed, components, dependencies, &mut visited, &mut ordered)?;
    }
    Ok(ordered)
}

fn render_field(field: &AvroField, render_type: &mut impl FnMut(&AvroType) -> Value) -> Value {
    let mut object = properties_object(field.properties());
    object.insert("name".to_owned(), Value::String(field.name().to_owned()));
    object.insert("type".to_owned(), render_type(field.tpe()));
    Value::Object(object)
}

fn decorate_type(value: Value, properties: &Properties, logical_type: Option<&str>) -> Value {
    let mut object = properties_object(properties);
    if let Some(logical_type) = logical_type {
        object.insert(
            "logicalType".to_owned(),
            Value::String(logical_type.to_owned()),
        );
    }
    match value {
        Value::Object(inner) => object.extend(inner),
        other => {
            object.insert("type".to_owned(), other);
        }
    }
    Value::Object(object)
}

fn decorate_root(value: Value, root: &AvroRoot, nested_named_target: bool) -> Value {
    let mut object = match (nested_named_target, value) {
        (true, nested) => {
            let mut object = Map::new();
            object.insert("type".to_owned(), nested);
            object
        }
        (false, Value::Object(inner)) => inner,
        (false, other) => {
            let mut object = Map::new();
            object.insert("type".to_owned(), other);
            object
        }
    };
    object.extend(properties_object(root.properties()));
    insert_doc(&mut object, root.doc());
    Value::Object(object)
}

fn properties_object(properties: &Properties) -> Map<String, Value> {
    properties
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn insert_doc(object: &mut Map<String, Value>, doc: Option<&str>) {
    if let Some(doc) = doc {
        object.insert("doc".to_owned(), Value::String(doc.to_owned()));
    }
}

fn insert_name(object: &mut Map<String, Value>, name: &AvroFullName) {
    object.insert("name".to_owned(), Value::String(name.name().to_owned()));
    if !name.namespace().is_empty() {
        object.insert(
            "namespace".to_owned(),
            Value::String(name.namespace().to_owned()),
        );
    }
}

fn schema_path(name: &AvroFullName) -> String {
    artifact_path(name, "avsc")
}

fn protocol_path(name: &AvroFullName) -> String {
    artifact_path(name, "avpr")
}

fn artifact_path(name: &AvroFullName, extension: &str) -> String {
    let mut components = name.namespace().split('.').collect::<Vec<_>>();
    components.retain(|component| !component.is_empty());
    components.push(name.name());
    format!("{}.{}", components.join("/"), extension)
}

fn text_artifact(path: String, value: Value) -> Result<Artifact, AvroDiagnostic> {
    let mut content = serde_json::to_string_pretty(&canonicalize(value))
        .map_err(|error| AvroDiagnostic::invalid_option(format!("render JSON: {error}")))?;
    content.push('\n');
    Ok(Artifact {
        path,
        content,
        binary: false,
    })
}

fn canonicalize(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize).collect()),
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, canonicalize(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        scalar => scalar,
    }
}

fn insert_artifact(
    artifacts: &mut Vec<Artifact>,
    paths: &mut BTreeSet<String>,
    artifact: Artifact,
) -> Result<(), AvroDiagnostic> {
    let path = artifact.path.clone();
    if !paths.insert(path.clone()) {
        return Err(AvroDiagnostic::name_collision(format!(
            "duplicate artifact path {path}"
        )));
    }
    artifacts.push(artifact);
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        AvroUnion,
        avro::{EnumSchema, FixedSchema},
    };

    #[test]
    fn renders_every_type_expression_form() {
        let package =
            AvroPackage::new(Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()).unwrap();
        let renderer = JsonRenderer::new(&package, Dependencies::SelfContained);
        let named = AvroFullName::new("example.types".to_owned(), "Named".to_owned()).unwrap();
        let cases = [
            (AvroType::Null, json!("null")),
            (AvroType::Boolean, json!("boolean")),
            (AvroType::Int, json!("int")),
            (AvroType::Long, json!("long")),
            (AvroType::Float, json!("float")),
            (AvroType::Double, json!("double")),
            (AvroType::Bytes, json!("bytes")),
            (AvroType::String, json!("string")),
            (
                AvroType::Array(Box::new(AvroType::String), Properties::new()),
                json!({"type": "array", "items": "string"}),
            ),
            (
                AvroType::Map(Box::new(AvroType::Long), Properties::new()),
                json!({"type": "map", "values": "long"}),
            ),
            (
                AvroType::Union(AvroUnion::new(vec![AvroType::Null, AvroType::String]).unwrap()),
                json!(["null", "string"]),
            ),
            (AvroType::Named(named), json!("example.types.Named")),
            (
                AvroType::Logical {
                    physical: Box::new(AvroType::Bytes),
                    name: "decimal".to_owned(),
                    properties: Properties::from([
                        ("precision".to_owned(), json!(12)),
                        ("scale".to_owned(), json!(2)),
                    ]),
                },
                json!({
                    "type": "bytes",
                    "logicalType": "decimal",
                    "precision": 12,
                    "scale": 2
                }),
            ),
            (
                AvroType::Annotated {
                    physical: Box::new(AvroType::String),
                    properties: Properties::from([("morphir.type-name".to_owned(), json!("Char"))]),
                },
                json!({"type": "string", "morphir.type-name": "Char"}),
            ),
        ];

        for (tpe, expected) in cases {
            assert_eq!(renderer.render_type_reference_only(&tpe), expected);
        }
    }

    #[test]
    fn renders_enum_and_fixed_standard_members_with_custom_properties() {
        let package =
            AvroPackage::new(Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()).unwrap();
        let renderer = JsonRenderer::new(&package, Dependencies::SelfContained);
        let enum_schema = NamedSchema::Enum(
            EnumSchema::new(
                AvroFullName::new("example.types".to_owned(), "Status".to_owned()).unwrap(),
                vec!["Pending".to_owned(), "Active".to_owned()],
                Some("Lifecycle status.".to_owned()),
                Properties::from([("morphir.source-kind".to_owned(), json!("custom"))]),
            )
            .unwrap(),
        );
        assert_eq!(
            renderer.render_named_reference_only(&enum_schema),
            json!({
                "type": "enum",
                "name": "Status",
                "namespace": "example.types",
                "symbols": ["Active", "Pending"],
                "doc": "Lifecycle status.",
                "morphir.source-kind": "custom"
            })
        );

        let fixed_schema = NamedSchema::Fixed(
            FixedSchema::new(
                AvroFullName::new("example.types".to_owned(), "Hash".to_owned()).unwrap(),
                32,
                Some("SHA-256 bytes.".to_owned()),
                Properties::from([("morphir.format".to_owned(), json!("sha-256"))]),
            )
            .unwrap(),
        );
        assert_eq!(
            renderer.render_named_reference_only(&fixed_schema),
            json!({
                "type": "fixed",
                "name": "Hash",
                "namespace": "example.types",
                "size": 32,
                "doc": "SHA-256 bytes.",
                "morphir.format": "sha-256"
            })
        );
    }

    #[test]
    fn canonicalization_sorts_nested_object_keys_without_reordering_arrays() {
        let value = Value::Object(Map::from_iter([
            (
                "z".to_owned(),
                Value::Object(Map::from_iter([
                    ("second".to_owned(), json!(2)),
                    ("first".to_owned(), json!(1)),
                ])),
            ),
            ("a".to_owned(), json!([{"z": 1, "a": 2}, "last"])),
        ]));
        let rendered = serde_json::to_string(&canonicalize(value)).unwrap();
        assert_eq!(
            rendered,
            r#"{"a":[{"a":2,"z":1},"last"],"z":{"first":1,"second":2}}"#
        );
    }
}

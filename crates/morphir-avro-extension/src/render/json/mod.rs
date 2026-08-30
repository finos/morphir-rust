mod graph;
mod syntax;
#[cfg(test)]
mod tests;

use self::{graph::*, syntax::*};
use std::collections::{BTreeMap, BTreeSet};

use morphir_extension_sdk::Artifact;
use serde_json::{Number, Value};

use crate::{
    AvroDiagnostic, AvroField, AvroFullName, AvroGenerationError, AvroInternalError, AvroPackage,
    AvroRoot, AvroType, Dependencies, NamedSchema, Protocol,
};

/// Render a checked Avro package as deterministic Avro JSON artifacts.
///
/// Every schema root gets its own `.avsc` artifact. Protocol projection emits
/// one `.avpr` artifact per module. Linked declarations are emitted once as
/// separate `.avsc` artifacts.
///
/// ```
/// use morphir_avro_extension::{
///     AvroOptions, Dependencies, DistributionKind, ProjectionPackage, project, render_json,
/// };
///
/// let source = ProjectionPackage {
///     kind: DistributionKind::Library,
///     package_name: "example".to_owned(),
///     dependencies: Vec::new(),
///     modules: Vec::new(),
/// };
/// let package = project(&source, &AvroOptions::default())?;
/// let artifacts = render_json(&package, Dependencies::SelfContained)?;
/// assert!(artifacts.is_empty());
/// # Ok::<(), morphir_avro_extension::AvroGenerationError>(())
/// ```
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
        for root in self.package.roots() {
            if (!self.package.protocols().is_empty() && matches!(root.tpe(), AvroType::Named(_)))
                || paths.contains(&schema_path(root.full_name()))
            {
                continue;
            }
            let artifact = self.render_root(root, &predefined)?;
            insert_artifact(&mut artifacts, &mut paths, artifact)?;
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

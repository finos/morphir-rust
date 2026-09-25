//! Authored metadata containers in the proposed 4.1.0 JSON and YAML profiles.
//!
//! These types preserve the authored spelling. Context expansion and schema-closure
//! validation are separate steps; retaining a spelling never implies that a
//! predicate or its data has been semantically validated.

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

use crate::metadata::{Coercion, ContextResources, EffectiveContext, resolve_context};
use crate::node_address::NodeUri;

/// A bounded authored context, validated against the inline context grammar.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthoredContext(Value);

impl AuthoredContext {
    /// Parse an inline context without loading external resources.
    pub fn parse(value: Value) -> Result<Self, String> {
        resolve_context(None, &value, &ContextResources::new("contexts"), None)
            .map_err(|error| error.to_string())?;
        Ok(Self(value))
    }

    /// The original context spelling.
    pub fn authored(&self) -> &Value {
        &self.0
    }

    fn effective(&self) -> EffectiveContext {
        resolve_context(None, &self.0, &ContextResources::new("contexts"), None)
            .expect("authored context was validated at construction")
    }
}

impl Serialize for AuthoredContext {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AuthoredContext {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// Authored fact properties with validated keys and node-link spelling.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AuthoredFacts(IndexMap<String, Value>);

impl AuthoredFacts {
    /// Parse fact properties using only this carrier's context.
    pub fn parse(value: &Value, context: Option<&AuthoredContext>) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "facts must be an object".to_owned())?;
        let effective = context.map(AuthoredContext::effective).unwrap_or_default();
        let mut facts = IndexMap::new();
        for (key, value) in object {
            let expanded = effective
                .expand_key(key)
                .map_err(|error| error.to_string())?;
            let values = match value {
                Value::Array(items) => items.iter().collect::<Vec<_>>(),
                _ => vec![value],
            };
            if expanded.coercion() == Coercion::NodeId {
                for item in values {
                    let uri = item
                        .as_str()
                        .ok_or_else(|| format!("{key} requires a node URI string"))?;
                    NodeUri::parse(uri).map_err(|error| error.to_string())?;
                }
            }
            facts.insert(key.clone(), value.clone());
        }
        Ok(Self(facts))
    }

    /// The preserved authored properties. A bare array denotes repeated objects.
    pub fn properties(&self) -> &IndexMap<String, Value> {
        &self.0
    }

    /// Whether no properties were authored.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Serialize for AuthoredFacts {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

/// One node-local or specification-local metadata scope.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MetadataScope {
    /// The scope's independent compact-name bindings.
    #[serde(rename = "@context", skip_serializing_if = "Option::is_none")]
    pub context: Option<AuthoredContext>,
    /// Its authored fact properties.
    #[serde(default, skip_serializing_if = "AuthoredFacts::is_empty")]
    pub facts: AuthoredFacts,
}

impl MetadataScope {
    /// Parse a context and facts together, so every fact key is checked against its own scope.
    pub fn parse(context: Option<&Value>, facts: Option<&Value>) -> Result<Self, String> {
        let context = context.cloned().map(AuthoredContext::parse).transpose()?;
        let facts = facts
            .map(|value| AuthoredFacts::parse(value, context.as_ref()))
            .transpose()?
            .unwrap_or_default();
        Ok(Self { context, facts })
    }

    /// Whether this scope has neither bindings nor facts.
    pub fn is_empty(&self) -> bool {
        self.context.is_none() && self.facts.is_empty()
    }

    /// Resolve a compact annotation or fact key in this scope.
    pub fn expand_key(&self, key: &str) -> Result<NodeUri, String> {
        self.context
            .as_ref()
            .map(AuthoredContext::effective)
            .unwrap_or_default()
            .expand_key(key)
            .map(|expanded| expanded.uri().clone())
            .map_err(|error| error.to_string())
    }
}

impl<'de> Deserialize<'de> for MetadataScope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("metadata scope must be an object"))?;
        for key in object.keys() {
            if !matches!(key.as_str(), "@context" | "facts") {
                return Err(serde::de::Error::custom(format!(
                    "unknown metadata member {key}"
                )));
            }
        }
        Self::parse(object.get("@context"), object.get("facts")).map_err(serde::de::Error::custom)
    }
}

/// One explicitly addressed subject in a document's default graph.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphSubject {
    id: String,
    facts: AuthoredFacts,
}

impl GraphSubject {
    /// The canonical subject address.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Authored fact properties on this subject.
    pub fn facts(&self) -> &AuthoredFacts {
        &self.facts
    }

    fn parse(value: &Value, context: Option<&AuthoredContext>) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "@graph entries must be objects".to_owned())?;
        let id = object
            .get("@id")
            .and_then(Value::as_str)
            .ok_or_else(|| "@graph entry requires a string @id".to_owned())?;
        NodeUri::parse(id).map_err(|error| error.to_string())?;
        let properties = object
            .iter()
            .filter(|(key, _)| key.as_str() != "@id")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Map<_, _>>();
        Ok(Self {
            id: id.to_owned(),
            facts: AuthoredFacts::parse(&Value::Object(properties), context)?,
        })
    }
}

impl Serialize for GraphSubject {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut object = Map::new();
        object.insert("@id".to_owned(), Value::String(self.id.clone()));
        for (key, value) in self.facts.properties() {
            object.insert(key.clone(), value.clone());
        }
        object.serialize(serializer)
    }
}

/// One authored selector and its nonempty source claims.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssertionSourceRecord {
    /// Expanded assertion identity in the containing document.
    pub selector: AssertionSelector,
    /// Explicit descriptive source claims.
    pub sources: Vec<SourceClaim>,
}

/// The expanded identity of one authored assertion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssertionSelector {
    /// `attributesFacts`, `annotationsFacts`, or `documentGraph`.
    pub carrier: String,
    /// Expanded canonical subject URI.
    pub subject: String,
    /// Expanded canonical predicate URI.
    pub predicate: String,
    /// A `@value` or `@id` object term.
    pub object: Value,
}

/// One document, compiler, or author source claim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum SourceClaim {
    /// Explicitly include the containing document.
    Document,
    /// A compiler and optional source reference.
    Compiler {
        producer: String,
        #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
        reference: Option<String>,
    },
    /// A human-supplied author reference.
    Author {
        #[serde(rename = "ref")]
        reference: String,
    },
}

/// Document-owned metadata for one 4.1.0 file.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DocumentMeta {
    /// Bindings for this document graph alone.
    #[serde(rename = "@context", skip_serializing_if = "Option::is_none")]
    pub context: Option<AuthoredContext>,
    /// Explicit subjects and their authored fact properties.
    #[serde(rename = "@graph", default, skip_serializing_if = "Vec::is_empty")]
    pub graph: Vec<GraphSubject>,
    /// Optional source details selected by expanded assertion identity.
    #[serde(
        rename = "assertionSources",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub assertion_sources: Vec<AssertionSourceRecord>,
}

impl DocumentMeta {
    /// Decode and validate the document container without loading context resources.
    pub fn parse(value: &Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "$meta must be an object".to_owned())?;
        for key in object.keys() {
            if !matches!(key.as_str(), "@context" | "@graph" | "assertionSources") {
                return Err(format!("unknown $meta member {key}"));
            }
        }
        let context = object
            .get("@context")
            .cloned()
            .map(AuthoredContext::parse)
            .transpose()?;
        let graph = object
            .get("@graph")
            .map(|value| {
                value
                    .as_array()
                    .ok_or_else(|| "@graph must be an array".to_owned())?
                    .iter()
                    .map(|item| GraphSubject::parse(item, context.as_ref()))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        let assertion_sources: Vec<AssertionSourceRecord> = object
            .get("assertionSources")
            .map(|value| serde_json::from_value(value.clone()).map_err(|error| error.to_string()))
            .transpose()?
            .unwrap_or_default();
        let mut selectors = BTreeSet::new();
        let effective = context
            .as_ref()
            .map(AuthoredContext::effective)
            .unwrap_or_default();
        for record in &assertion_sources {
            if record.sources.is_empty() {
                return Err("assertionSources entries require at least one source".to_owned());
            }
            if record.selector.carrier != "documentGraph" {
                return Err(
                    "node-carrier assertion source selectors require owner resolution".to_owned(),
                );
            }
            NodeUri::parse(&record.selector.subject).map_err(|error| error.to_string())?;
            NodeUri::parse(&record.selector.predicate).map_err(|error| error.to_string())?;
            let object = record
                .selector
                .object
                .as_object()
                .ok_or_else(|| "selector object must be @value or @id".to_owned())?;
            if object.len() != 1 || (!object.contains_key("@value") && !object.contains_key("@id"))
            {
                return Err("selector object must be @value or @id".to_owned());
            }
            if let Some(id) = object.get("@id") {
                let id = id
                    .as_str()
                    .ok_or_else(|| "selector @id must be a string".to_owned())?;
                NodeUri::parse(id).map_err(|error| error.to_string())?;
            }
            let identity =
                serde_json::to_string(&record.selector).map_err(|error| error.to_string())?;
            if !selectors.insert(identity) {
                return Err("duplicate assertion source selector".to_owned());
            }
            if record.selector.carrier == "documentGraph" {
                let matched = graph
                    .iter()
                    .filter(|subject| subject.id() == record.selector.subject)
                    .any(|subject| {
                        subject.facts().properties().iter().any(|(key, authored)| {
                            let Ok(expanded) = effective.expand_key(key) else {
                                return false;
                            };
                            if expanded.uri().to_string() != record.selector.predicate {
                                return false;
                            }
                            let values = match authored {
                                Value::Array(items) => items.iter().collect::<Vec<_>>(),
                                _ => vec![authored],
                            };
                            values.into_iter().any(|value| {
                                let object = if expanded.coercion() == Coercion::NodeId {
                                    serde_json::json!({"@id": value})
                                } else {
                                    serde_json::json!({"@value": value})
                                };
                                object == record.selector.object
                            })
                        })
                    });
                if !matched {
                    return Err(
                        "assertion source selector has no matching document-graph assertion"
                            .to_owned(),
                    );
                }
            }
        }
        Ok(Self {
            context,
            graph,
            assertion_sources,
        })
    }
}

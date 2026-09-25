//! Expansion of one document-owned V4 metadata graph into semantic assertions.
//!
//! This step preserves carrier and source identity. It does not validate
//! predicate declarations, target nodes, or closed Morphir data shapes.

use super::IRFile;
use super::linked_metadata::{AssertionSourceRecord, DocumentMeta, SourceClaim};
use crate::metadata::Carrier;
use crate::metadata::{
    Assertion, AssertionKey, AssertionSource, ContextError, ContextResources, DocumentId, Fact,
    GraphIndex, GraphName, MetadataError, ObjectTerm, SourceRecord, expand_properties,
    resolve_context,
};
use crate::node_address::{IndexedNodeKind, NodeIndex, NodeResolutionError, NodeUri};
use serde_json::Value;

/// A malformed authored graph or source selector.
#[derive(Debug, thiserror::Error)]
pub enum DocumentGraphError {
    #[error(transparent)]
    Context(#[from] ContextError),
    #[error(transparent)]
    Model(#[from] MetadataError),
    #[error(transparent)]
    Index(#[from] NodeResolutionError),
    #[error("invalid subject or predicate node URI: {0}")]
    InvalidNode(String),
    #[error("invalid assertion source selector object")]
    InvalidSelectorObject,
}

/// Expand all authored carriers in one V4 single-file document. Each indexed
/// Type/Value/Pattern occurrence supplies its own implicit subject; specs
/// additionally supply independent annotation scopes. This retains the one
/// file owner and does not yet validate declaration roles or external targets.
pub fn expand_v4_single_file_graph(
    file: &IRFile,
    owner: &DocumentId,
    resources: &ContextResources,
    json_datatype: impl Fn(&NodeUri) -> Option<NodeUri>,
) -> Result<GraphIndex, DocumentGraphError> {
    let mut graph = if let Some(meta) = &file.metadata {
        expand_document_graph(meta, owner, resources, &json_datatype)?
    } else {
        GraphIndex::new()
    };
    let parent = file
        .metadata
        .as_ref()
        .and_then(|meta| meta.context.as_ref())
        .map(|context| resolve_context(None, context.authored(), resources, None))
        .transpose()?
        .unwrap_or_default();
    let index = NodeIndex::v4_file(file)?;
    let mut nodes = index.nodes().collect::<Vec<_>>();
    nodes.sort_by_key(|(uri, _)| uri.to_string());
    for (subject, node) in nodes {
        match node.kind {
            IndexedNodeKind::TypeExpression
            | IndexedNodeKind::ValueExpression
            | IndexedNodeKind::Pattern => {
                if let Some(scope) = tagged_payload(&node.semantic_value)
                    .and_then(|payload| payload.get("attributes"))
                {
                    insert_scope(
                        &mut graph,
                        scope,
                        subject,
                        owner,
                        &parent,
                        resources,
                        Carrier::AttributesFacts(subject.clone()),
                        &json_datatype,
                    )?;
                }
            }
            IndexedNodeKind::Module
            | IndexedNodeKind::TypeDefinition
            | IndexedNodeKind::ValueDefinition => {
                let scope = node.semantic_value.get("annotations").or_else(|| {
                    tagged_payload(&node.semantic_value)
                        .and_then(|payload| payload.get("annotations"))
                });
                if let Some(scope) = scope.filter(|scope| scope.is_object()) {
                    insert_scope(
                        &mut graph,
                        scope,
                        subject,
                        owner,
                        &parent,
                        resources,
                        Carrier::AnnotationsFacts(subject.clone()),
                        &json_datatype,
                    )?;
                }
            }
            _ => {}
        }
    }
    Ok(graph)
}

fn tagged_payload(value: &Value) -> Option<&Value> {
    let object = value.as_object()?;
    (object.len() == 1)
        .then(|| object.values().next())
        .flatten()
}

#[allow(clippy::too_many_arguments)]
fn insert_scope(
    graph: &mut GraphIndex,
    scope: &Value,
    subject: &NodeUri,
    owner: &DocumentId,
    parent: &crate::metadata::EffectiveContext,
    resources: &ContextResources,
    carrier: Carrier,
    json_datatype: &impl Fn(&NodeUri) -> Option<NodeUri>,
) -> Result<(), DocumentGraphError> {
    let Some(scope) = scope.as_object() else {
        return Ok(());
    };
    let context = scope
        .get("@context")
        .map(|value| resolve_context(Some(parent), value, resources, None))
        .transpose()?
        .unwrap_or_else(|| parent.clone());
    let Some(properties) = scope.get("facts").and_then(Value::as_object) else {
        return Ok(());
    };
    for fact in expand_properties(
        subject,
        properties.iter().map(|(key, value)| (key.as_str(), value)),
        &context,
        json_datatype,
    )? {
        let key = AssertionKey::new(owner.clone(), carrier.clone(), fact)?;
        graph.insert(Assertion::new(key))?;
    }
    Ok(())
}

/// Expand `$meta.@graph` and its optional detailed-source table.
///
/// `json_datatype` comes from the caller's verified predicate declaration
/// closure. Supplying it only determines object identity here; validation of
/// the underlying Morphir type and any interpreter remains a later step.
pub fn expand_document_graph(
    metadata: &DocumentMeta,
    owner: &DocumentId,
    resources: &ContextResources,
    json_datatype: impl Fn(&NodeUri) -> Option<NodeUri>,
) -> Result<GraphIndex, DocumentGraphError> {
    let context = metadata
        .context
        .as_ref()
        .map(|value| resolve_context(None, value.authored(), resources, None))
        .transpose()?
        .unwrap_or_default();
    let mut graph = GraphIndex::new();
    for subject in &metadata.graph {
        let uri = parse_node(subject.id())?;
        for fact in expand_properties(
            &uri,
            subject
                .facts()
                .properties()
                .iter()
                .map(|(key, value)| (key.as_str(), value)),
            &context,
            &json_datatype,
        )? {
            let key =
                AssertionKey::new(owner.clone(), crate::metadata::Carrier::DocumentGraph, fact)?;
            graph.insert(Assertion::new(key))?;
        }
    }
    let records = metadata
        .assertion_sources
        .iter()
        .map(|record| source_record(record, owner, &json_datatype))
        .collect::<Result<Vec<_>, _>>()?;
    graph.apply_source_records(owner, &records)?;
    Ok(graph)
}

fn source_record(
    record: &AssertionSourceRecord,
    owner: &DocumentId,
    json_datatype: &impl Fn(&NodeUri) -> Option<NodeUri>,
) -> Result<SourceRecord, DocumentGraphError> {
    if record.selector.carrier != "documentGraph" {
        return Err(DocumentGraphError::InvalidSelectorObject);
    }
    let subject = parse_node(&record.selector.subject)?;
    let predicate = parse_node(&record.selector.predicate)?;
    let object = selector_object(&record.selector.object, &predicate, json_datatype)?;
    let fact = Fact::new(subject, predicate, object, GraphName::Default);
    let key = AssertionKey::new(owner.clone(), crate::metadata::Carrier::DocumentGraph, fact)?;
    let sources = record
        .sources
        .iter()
        .map(|source| match source {
            SourceClaim::Document => AssertionSource::Document(owner.clone()),
            SourceClaim::Compiler {
                producer,
                reference,
            } => AssertionSource::Compiler {
                producer: producer.clone(),
                reference: reference.clone(),
            },
            SourceClaim::Author { reference } => AssertionSource::Author {
                reference: reference.clone(),
            },
        })
        .collect();
    Ok(SourceRecord::new(key, sources)?)
}

fn selector_object(
    value: &Value,
    predicate: &NodeUri,
    json_datatype: &impl Fn(&NodeUri) -> Option<NodeUri>,
) -> Result<ObjectTerm, DocumentGraphError> {
    let object = value
        .as_object()
        .ok_or(DocumentGraphError::InvalidSelectorObject)?;
    if object.len() == 1 {
        if let Some(id) = object.get("@id").and_then(Value::as_str) {
            return Ok(ObjectTerm::NodeRef(parse_node(id)?));
        }
        if let Some(value) = object.get("@value") {
            return Ok(ObjectTerm::value(value.clone()));
        }
    }
    if object.len() == 2
        && object.get("@type") == Some(&Value::String("@json".to_owned()))
        && let Some(value) = object.get("@value")
    {
        let datatype = json_datatype(predicate).ok_or(ContextError::MissingJsonDatatype)?;
        return Ok(ObjectTerm::typed_json(value.clone(), datatype));
    }
    Err(DocumentGraphError::InvalidSelectorObject)
}

fn parse_node(value: &str) -> Result<NodeUri, DocumentGraphError> {
    NodeUri::parse(value).map_err(|error| DocumentGraphError::InvalidNode(error.to_string()))
}

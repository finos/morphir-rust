//! Expanded fact terms and graph identity.

use crate::node_address::NodeUri;
use serde_json::Value;

/// The graph containing a fact. Only [`GraphName::Default`] executes today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphName {
    /// The default graph used by the first executable increment.
    Default,
    /// A future named graph, retained as a distinct logical identity.
    Named(NodeUri),
}

/// Data kept apart from a node reference.
///
/// This is a normalized storage term. Callers must validate the value against
/// its predicate declaration before claiming it has validated semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedValue {
    value: Value,
    datatype: Option<NodeUri>,
}

impl TypedValue {
    /// A data value without an explicit `@json` type.
    pub fn new(value: Value) -> Self {
        Self {
            value,
            datatype: None,
        }
    }

    /// A single structured `@json` object with its expanded type declaration.
    /// This constructor does not validate the declaration's closed data shape.
    pub fn json(value: Value, datatype: NodeUri) -> Self {
        Self {
            value,
            datatype: Some(datatype),
        }
    }

    /// The closed data tree. A URI-looking string in this tree remains data.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// The expanded type declaration for an `@json` value, if present.
    pub fn datatype(&self) -> Option<&NodeUri> {
        self.datatype.as_ref()
    }

    pub(crate) fn identity(&self) -> String {
        serde_json::to_string(&(
            self.datatype.as_ref().map(ToString::to_string),
            canonical_value(&self.value),
        ))
        .expect("JSON values and strings serialize")
    }
}

/// One object of an expanded fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectTerm {
    /// Data, including literal strings that resemble node URIs.
    Value(TypedValue),
    /// A semantic link to another addressed IR node.
    NodeRef(NodeUri),
}

impl ObjectTerm {
    /// Construct a data object without `@json` coercion.
    pub fn value(value: Value) -> Self {
        Self::Value(TypedValue::new(value))
    }

    /// Construct one typed `@json` data object.
    pub fn typed_json(value: Value, datatype: NodeUri) -> Self {
        Self::Value(TypedValue::json(value, datatype))
    }

    pub(crate) fn identity(&self) -> String {
        match self {
            Self::Value(value) => format!("value:{}", value.identity()),
            Self::NodeRef(uri) => format!("node:{}", uri),
        }
    }
}

/// One expanded subject-predicate-object-graph value.
///
/// A graph query returns a fact once even if several documents or carriers
/// asserted it. Ownership belongs to [`crate::metadata::AssertionKey`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    subject: NodeUri,
    predicate: NodeUri,
    object: ObjectTerm,
    graph: GraphName,
}

impl Fact {
    /// Construct a fact from expanded semantic identities.
    pub fn new(subject: NodeUri, predicate: NodeUri, object: ObjectTerm, graph: GraphName) -> Self {
        Self {
            subject,
            predicate,
            object,
            graph,
        }
    }

    /// The addressed subject node.
    pub fn subject(&self) -> &NodeUri {
        &self.subject
    }

    /// The expanded predicate declaration address.
    pub fn predicate(&self) -> &NodeUri {
        &self.predicate
    }

    /// The typed data or node-reference object.
    pub fn object(&self) -> &ObjectTerm {
        &self.object
    }

    /// The logical graph identity.
    pub fn graph(&self) -> &GraphName {
        &self.graph
    }

    pub(crate) fn identity(&self) -> String {
        serde_json::to_string(&(
            self.subject.to_string(),
            self.predicate.to_string(),
            self.object.identity(),
            match &self.graph {
                GraphName::Default => "default".to_owned(),
                GraphName::Named(uri) => format!("named:{uri}"),
            },
        ))
        .expect("fact identities serialize")
    }
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonical_value).collect()),
        Value::Object(members) => {
            let mut keys = members.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut sorted = serde_json::Map::new();
            for key in keys {
                sorted.insert(key.clone(), canonical_value(&members[key]));
            }
            Value::Object(sorted)
        }
        _ => value.clone(),
    }
}

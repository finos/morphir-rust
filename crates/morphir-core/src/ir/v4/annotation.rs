//! Annotations on specifications (definitions-0020 to 0023).
//!
//! An annotation names a value in the public face of a type, a value or a module. The compact
//! spelling is the string `pkg:mod#local` or `pkg:mod#local:free text`, where the separator is the
//! first colon after the local-name hash; the structured spelling is `{ "name", "arguments" }`,
//! whose arguments are positional value expressions or named `{ "name", "value" }` pairs.
//! Definitions never carry annotations.

use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Deserializer, Serialize};
use std::ops::Deref;

use super::linked_metadata::MetadataScope;
use super::linked_metadata_scan::StandaloneMetadata;
use super::value::Value;
use crate::naming::{FQName, Name};
use crate::node_address::NodeUri;

/// A specification's annotations and optional independent fact scope.
///
/// The old array spelling is retained when the metadata scope is empty. The
/// 4.1.0 envelope writes `entries` beside `@context` and `facts`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Annotations {
    /// Existing named entries and their arguments.
    pub entries: Vec<Annotation>,
    /// Scoped authored facts on the enclosing specification.
    pub metadata: Option<Box<MetadataScope>>,
}

impl Annotations {
    /// Retain an authored envelope until the containing document's context is known.
    /// Callers must validate the completed IR file before exposing it.
    pub fn parse_unresolved(value: &serde_json::Value) -> Result<Self, String> {
        super::serde_document::decode_annotations_value(value, "annotations")
            .map_err(|error| error.message)
    }

    /// Make an ordinary annotation array with no linked facts.
    pub fn new(entries: Vec<Annotation>) -> Self {
        Self {
            entries,
            metadata: None,
        }
    }

    /// Whether neither entries nor metadata were authored.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.metadata.is_none()
    }
}

impl Deref for Annotations {
    type Target = [Annotation];

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl From<Vec<Annotation>> for Annotations {
    fn from(entries: Vec<Annotation>) -> Self {
        Self::new(entries)
    }
}

impl PartialEq<Vec<Annotation>> for Annotations {
    fn eq(&self, other: &Vec<Annotation>) -> bool {
        self.metadata.is_none() && self.entries == *other
    }
}

impl Serialize for Annotations {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Some(metadata) = &self.metadata {
            let mut map = serializer.serialize_map(None)?;
            if let Some(context) = &metadata.context {
                map.serialize_entry("@context", context)?;
            }
            if !self.entries.is_empty() {
                map.serialize_entry("entries", &self.entries)?;
            }
            if !metadata.facts.is_empty() {
                map.serialize_entry("facts", &metadata.facts)?;
            }
            map.end()
        } else {
            self.entries.serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for Annotations {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let mut decoded = super::serde_document::decode_annotations_value(&value, "")
            .map_err(super::serde_tagged::carry)?;
        decoded
            .validate_standalone()
            .map_err(serde::de::Error::custom)?;
        Ok(decoded)
    }
}

/// An annotation on a type, value or module specification.
#[derive(Debug, Clone, PartialEq)]
pub enum Annotation {
    /// The compact spelling: a name, and optionally the free text after it.
    Compact { name: FQName, text: Option<String> },
    /// The structured spelling: a name and its arguments, written only when it has some.
    Structured {
        name: FQName,
        args: Vec<AnnotationArgument>,
    },
    /// A 4.1.0 compact entry resolved through `annotations.@context`.
    LinkedCompact {
        /// The authored alias or compact IRI.
        authored_name: String,
        /// Expanded declaration identity.
        declaration: NodeUri,
    },
    /// A 4.1.0 structured entry with preserved positional and named arguments.
    LinkedStructured {
        /// The authored alias or compact IRI.
        authored_name: String,
        /// Expanded declaration identity.
        declaration: NodeUri,
        /// The existing argument vocabulary.
        args: Vec<AnnotationArgument>,
    },
    /// An alias awaiting the enclosing document's context during whole-file decode.
    PendingCompact { authored_name: String },
    /// A structured alias awaiting the enclosing document's context.
    PendingStructured {
        authored_name: String,
        args: Vec<AnnotationArgument>,
    },
}

/// An argument of a structured annotation.
#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationArgument {
    /// A value expression written on its own.
    Positional(Value),
    /// A value expression written under the name it answers to.
    Named { name: Name, value: Value },
}

impl Serialize for Annotation {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Annotation::Compact { name, text: None } => {
                serializer.serialize_str(&name.to_canonical_string())
            }
            Annotation::Compact {
                name,
                text: Some(text),
            } => serializer.serialize_str(&format!("{}:{text}", name.to_canonical_string())),
            Annotation::Structured { name, args } => {
                let mut map = serializer.serialize_map(None)?;
                map.serialize_entry("name", &name.to_canonical_string())?;
                if !args.is_empty() {
                    map.serialize_entry("arguments", args)?;
                }
                map.end()
            }
            Annotation::LinkedCompact { authored_name, .. }
            | Annotation::PendingCompact { authored_name } => {
                serializer.serialize_str(authored_name)
            }
            Annotation::LinkedStructured {
                authored_name,
                args,
                ..
            }
            | Annotation::PendingStructured {
                authored_name,
                args,
            } => {
                let mut map = serializer.serialize_map(None)?;
                map.serialize_entry("name", authored_name)?;
                if !args.is_empty() {
                    map.serialize_entry("arguments", args)?;
                }
                map.end()
            }
        }
    }
}

impl Serialize for AnnotationArgument {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            AnnotationArgument::Positional(value) => value.serialize(serializer),
            AnnotationArgument::Named { name, value } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("name", &name.to_canonical_string())?;
                map.serialize_entry("value", value)?;
                map.end()
            }
        }
    }
}

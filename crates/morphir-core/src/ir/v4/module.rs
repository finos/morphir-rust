//! Module types for Morphir IR V4
//!
//! This module contains ModuleSpecification, ModuleDefinition, and related types.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::access::AccessControlled;
use super::types::{TypeDefinition, TypeSpecification};
use super::value::{ValueDefinition, ValueSpecification};

/// Documentation stored as normalized lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Documentation(Vec<String>);

impl Documentation {
    /// Construct documentation from an iterator of lines.
    pub fn new(lines: impl IntoIterator<Item = String>) -> Self {
        Self(lines.into_iter().map(normalize_line).collect())
    }

    /// Return the normalized documentation lines.
    pub fn lines(&self) -> &[String] {
        &self.0
    }
}

fn normalize_line(line: String) -> String {
    line.strip_suffix('\r').unwrap_or(&line).to_owned()
}

impl From<String> for Documentation {
    fn from(value: String) -> Self {
        Self::new([value])
    }
}

impl Serialize for Documentation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self.0.as_slice() {
            [line] => serializer.serialize_str(line),
            lines => lines.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Documentation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Line(String),
            Lines(Vec<String>),
        }

        match Repr::deserialize(deserializer)? {
            Repr::Line(line) => Ok(Self::new([line])),
            Repr::Lines(lines) => Ok(Self::new(lines)),
        }
    }
}

/// Optional documentation paired with an IR definition or specification.
#[derive(Debug, Clone, PartialEq)]
pub struct Documented<T> {
    pub doc: Option<Documentation>,
    pub value: T,
}

impl<T> Documented<T> {
    pub fn new(doc: Option<Documentation>, value: T) -> Self {
        Self { doc, value }
    }
}

impl<T: Serialize> Serialize for Documented<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let Some(doc) = &self.doc else {
            return self.value.serialize(serializer);
        };

        // Decision 0010: `doc` is a flattened member placed first beside the documented node's
        // own members, rather than a wrapper the node sits under.
        let written = serde_json::to_value(&self.value).map_err(serde::ser::Error::custom)?;
        let doc = serde_json::to_value(doc).map_err(serde::ser::Error::custom)?;
        match written {
            serde_json::Value::Object(members) => {
                let mut flattened = serde_json::Map::with_capacity(members.len() + 1);
                flattened.insert("doc".to_owned(), doc);
                flattened.extend(members);
                serde_json::Value::Object(flattened).serialize(serializer)
            }
            other => {
                let mut wrapper = serde_json::Map::with_capacity(2);
                wrapper.insert("doc".to_owned(), doc);
                wrapper.insert("value".to_owned(), other);
                serde_json::Value::Object(wrapper).serialize(serializer)
            }
        }
    }
}

impl<'de, T> Deserialize<'de> for Documented<T>
where
    T: for<'value> Deserialize<'value>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use super::serde_document::{carried, recover};

        let value = serde_json::Value::deserialize(deserializer)?;
        super::serde_document::decode_documented(&value, "", |payload, cursor| {
            serde_json::from_value::<T>(payload.clone()).map_err(|error| recover(&error, cursor))
        })
        .map_err(carried)
    }
}

/// Module specification (public API only)
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleSpecification {
    pub types: IndexMap<String, Documented<TypeSpecification>>,
    pub values: IndexMap<String, Documented<ValueSpecification>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<Documentation>,
}

impl<'de> Deserialize<'de> for ModuleSpecification {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        super::serde_document::deserialize_with(
            deserializer,
            super::serde_document::decode_module_specification,
        )
    }
}

/// Module definition
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleDefinition {
    pub types: IndexMap<String, AccessControlled<Documented<TypeDefinition>>>,
    pub values: IndexMap<String, AccessControlled<Documented<ValueDefinition>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<Documentation>,
}

impl<'de> Deserialize<'de> for ModuleDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        super::serde_document::deserialize_with(
            deserializer,
            super::serde_document::decode_module_definition,
        )
    }
}

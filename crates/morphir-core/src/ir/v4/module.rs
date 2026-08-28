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
        if let Some(doc) = &self.doc {
            #[derive(Serialize)]
            struct Repr<'a, T> {
                doc: &'a Documentation,
                value: &'a T,
            }
            Repr {
                doc,
                value: &self.value,
            }
            .serialize(serializer)
        } else {
            self.value.serialize(serializer)
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
        let value = serde_json::Value::deserialize(deserializer)?;
        if let Some(object) = value.as_object()
            && object.contains_key("doc")
            && object.contains_key("value")
        {
            let doc =
                serde_json::from_value(object["doc"].clone()).map_err(serde::de::Error::custom)?;
            let value = serde_json::from_value(object["value"].clone())
                .map_err(serde::de::Error::custom)?;
            return Ok(Self {
                doc: Some(doc),
                value,
            });
        }

        serde_json::from_value(value)
            .map(|value| Self { doc: None, value })
            .map_err(serde::de::Error::custom)
    }
}

/// Module specification (public API only)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleSpecification {
    pub types: IndexMap<String, Documented<TypeSpecification>>,
    pub values: IndexMap<String, Documented<ValueSpecification>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<Documentation>,
}

/// Module definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleDefinition {
    pub types: IndexMap<String, AccessControlled<Documented<TypeDefinition>>>,
    pub values: IndexMap<String, AccessControlled<Documented<ValueDefinition>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<Documentation>,
}

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

/// Module specification (public API only)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleSpecification {
    pub types: IndexMap<String, TypeSpecification>,
    pub values: IndexMap<String, ValueSpecification>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<Documentation>,
}

/// Module definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleDefinition {
    pub types: IndexMap<String, AccessControlled<TypeDefinition>>,
    pub values: IndexMap<String, AccessControlled<ValueDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<Documentation>,
}

//! Annotations on specifications (definitions-0020 to 0023).
//!
//! An annotation names a value in the public face of a type, a value or a module. The compact
//! spelling is the string `pkg:mod#local` or `pkg:mod#local:free text`, where the separator is the
//! first colon after the local-name hash; the structured spelling is `{ "name", "arguments" }`,
//! whose arguments are positional value expressions or named `{ "name", "value" }` pairs.
//! Definitions never carry annotations.

use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};

use super::value::Value;
use crate::naming::{FQName, Name};

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

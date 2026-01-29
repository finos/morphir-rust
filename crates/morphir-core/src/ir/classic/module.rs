//! Classic IR Module types
//!
//! Module structures for the Classic Morphir IR format.

use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

use super::access::AccessControlled;
use super::documented::Documented;
use super::naming::{Name, Path};
use super::types::TypeDefinition;
use super::value::ValueDefinition;

/// Module entry - [modulePath, AccessControlled<ModuleDefinition>]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModuleEntry {
    pub path: Path,
    pub definition: AccessControlled<ModuleDefinition>,
}

impl<'de> Deserialize<'de> for ModuleEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ModuleEntryVisitor;

        impl<'de> Visitor<'de> for ModuleEntryVisitor {
            type Value = ModuleEntry;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a ModuleEntry array [path, definition]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let path = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let definition = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                if let Some(_) = seq.next_element::<serde_json::Value>()? {
                    return Err(de::Error::custom("Expected end of ModuleEntry array"));
                }

                Ok(ModuleEntry { path, definition })
            }
        }

        deserializer.deserialize_seq(ModuleEntryVisitor)
    }
}

/// Module definition - the inner content of a module
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleDefinition {
    #[serde(default)]
    pub types: Vec<(
        Name,
        AccessControlled<Documented<TypeDefinition<serde_json::Value>>>,
    )>,
    #[serde(default)]
    pub values: Vec<(
        Name,
        AccessControlled<Documented<ValueDefinition<serde_json::Value, serde_json::Value>>>,
    )>,
}

// Keep the old Module struct for compatibility during migration
/// Module definition (legacy format)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Module {
    pub name: Path,
    pub detail: ModuleDetail,
}

/// Module details (legacy format)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleDetail {
    pub documentation: Option<String>,
    #[serde(default)]
    pub types: Vec<(
        Name,
        AccessControlled<Documented<TypeDefinition<serde_json::Value>>>,
    )>,
    #[serde(default)]
    pub values: Vec<(
        Name,
        AccessControlled<Documented<ValueDefinition<serde_json::Value, serde_json::Value>>>,
    )>,
}



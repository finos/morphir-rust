//! Package types for Morphir IR V4
//!
//! This module contains PackageDefinition, PackageSpecification, and related types.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::access::AccessControlled;
use super::module::{ModuleDefinition, ModuleSpecification};

/// Package specification (for dependencies)
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageSpecification {
    pub modules: IndexMap<String, ModuleSpecification>,
}

impl<'de> Deserialize<'de> for PackageSpecification {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        super::serde_document::deserialize_with(
            deserializer,
            super::serde_document::decode_package_specification,
        )
    }
}

/// Package definition
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageDefinition {
    pub modules: IndexMap<String, AccessControlled<ModuleDefinition>>,
}

impl<'de> Deserialize<'de> for PackageDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        super::serde_document::deserialize_with(
            deserializer,
            super::serde_document::decode_package_definition,
        )
    }
}

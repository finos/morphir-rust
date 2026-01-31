//! Package types for Morphir IR V4
//!
//! This module contains PackageDefinition, PackageSpecification, and related types.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::access::AccessControlled;
use super::module::{ModuleDefinition, ModuleSpecification};

/// Package specification (for dependencies)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageSpecification {
    pub modules: IndexMap<String, ModuleSpecification>,
}

/// Package definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageDefinition {
    pub modules: IndexMap<String, AccessControlled<ModuleDefinition>>,
}

//! Access control types for Morphir IR V4

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Access control
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum Access {
    Public,
    Private,
}

/// Generic wrapper for access-controlled values
///
/// This matches morphir-elm's AccessControlled type, which is a generic wrapper
/// that can be applied to any type that needs access control.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessControlled<T> {
    pub access: Access,
    pub value: T,
}

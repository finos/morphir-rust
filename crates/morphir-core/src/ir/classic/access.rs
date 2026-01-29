//! Classic IR Access Control types
//!
//! Access control wrappers for the Classic Morphir IR format.

use serde::{Deserialize, Serialize};

/// Access level - serialized as PascalCase ("Public" or "Private")
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Access {
    Public,
    Private,
}

/// Access controlled content - serialized as {"access":"Public/Private","value":...}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessControlled<A> {
    pub access: Access,
    pub value: A,
}

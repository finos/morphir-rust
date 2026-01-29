//! Classic IR Package types
//!
//! Package structures for the Classic Morphir IR format.

use serde::{Deserialize, Serialize};

use super::module::ModuleEntry;

/// Package definition - contains a list of module entries
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Package {
    pub modules: Vec<ModuleEntry>,
}


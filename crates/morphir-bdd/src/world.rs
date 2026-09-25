//! The one world type every Morphir suite runs with.

use std::sync::Arc;

use morphir_gherkin::extension::Context;
use morphir_gherkin::{Document, NodePath};

/// The running scenario: its document and its path in that document.
#[derive(Debug, Clone)]
pub struct ScenarioRef {
    pub document: Arc<Document>,
    pub path: NodePath,
}

/// The state of one scenario. Extensions fill `context` before the first step; steps read and add
/// components.
#[derive(Debug, cucumber::World)]
#[world(init = MorphirWorld::new)]
pub struct MorphirWorld {
    pub context: Context,
    pub scenario: Option<ScenarioRef>,
}

impl MorphirWorld {
    pub fn new() -> Self {
        Self {
            context: Context::default(),
            scenario: None,
        }
    }
}

impl Default for MorphirWorld {
    fn default() -> Self {
        Self::new()
    }
}

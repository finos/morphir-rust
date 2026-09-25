//! The one world type every Morphir suite runs with.

use std::sync::Arc;

use morphir_gherkin::extension::Context;
use morphir_gherkin::{Document, NodePath};

/// The running scenario: its document and its path in that document.
#[derive(Debug, Clone)]
pub struct ScenarioRef {
    /// The morphir-gherkin document the scenario was read from.
    pub document: Arc<Document>,
    /// The scenario's node path in [`ScenarioRef::document`].
    ///
    /// For a plain scenario this is its scenario path (`feature/scenario[i]`, or under a rule).
    /// For one expanded row of a scenario outline it is the path of that row's `Examples` block
    /// (`…/scenario[i]/examples[j]`): the context was built from that block's tags and
    /// description only, not from the outline's other blocks. Take
    /// [`NodePath::parent`] of it to reach the outline itself.
    pub path: NodePath,
}

/// The state of one scenario. Extensions fill `context` before the first step; steps read and add
/// components.
#[derive(Debug, cucumber::World)]
#[world(init = MorphirWorld::new)]
pub struct MorphirWorld {
    /// The typed components of this scenario: first what the suite's extensions built from its
    /// tags, fences and prose, then what its steps add (a workspace, the last command's output).
    pub context: Context,
    /// The running scenario's document and node path, or `None` before the suite's `before` hook
    /// has filled it (for example in a raw cucumber run with no Morphir hook).
    pub scenario: Option<ScenarioRef>,
}

impl MorphirWorld {
    /// An empty world: no context components and no running scenario yet.
    #[must_use]
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

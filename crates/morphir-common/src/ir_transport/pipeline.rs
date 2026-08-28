//! Contracts for composable semantic IR transforms.

use morphir_core::traversal::SemanticEvent;

use super::TransportDiagnostic;

/// Largest semantic scope retained by a transform.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Retention {
    /// Retains no context beyond the current event.
    Event,
    /// Retains one type or value definition.
    Definition,
    /// Retains one module.
    Module,
    /// Retains the complete distribution.
    Distribution,
}

/// Semantic rewrite that is independent of JSON, YAML, and physical layout.
pub trait EventTransform {
    /// Declare the largest semantic scope retained by this transform.
    fn retention(&self) -> Retention;

    /// Transform one input into zero, one, or many output events.
    fn transform(
        &mut self,
        event: SemanticEvent,
        emit: &mut dyn FnMut(SemanticEvent) -> Result<(), TransportDiagnostic>,
    ) -> Result<(), TransportDiagnostic>;
}

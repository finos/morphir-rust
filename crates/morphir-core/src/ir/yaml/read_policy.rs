//! Internal policies shared by the historical reader and the opt-in bounded route.
use crate::ir::Diagnostic;
use crate::ir::decode::{DecodeLimitExceeded, PhaseBudget};
use granit_parser::{ErrorKind, ScanError};
use morphir_json::{ReadBudget, ValueKind};
use std::convert::Infallible;

pub(crate) enum ReadFailure<E> {
    Profile(Diagnostic),
    Limit(E),
}
impl<E> From<Diagnostic> for ReadFailure<E> {
    fn from(error: Diagnostic) -> Self {
        Self::Profile(error)
    }
}
pub(super) trait YamlBudget: ReadBudget {
    fn parser_nesting_limit(&self) -> usize;
    fn scanner_refusal(&self, error: &ScanError) -> Option<Self::Error>;
}
pub(super) struct Legacy;
impl ReadBudget for Legacy {
    type Error = Infallible;
    fn before_value(&mut self, _: usize, _: ValueKind) -> Result<(), Infallible> {
        Ok(())
    }
    fn before_string(&mut self, _: &str) -> Result<(), Infallible> {
        Ok(())
    }
}
impl YamlBudget for Legacy {
    fn parser_nesting_limit(&self) -> usize {
        super::read::MAX_DEPTH + 8
    }
    fn scanner_refusal(&self, _: &ScanError) -> Option<Infallible> {
        None
    }
}
impl YamlBudget for PhaseBudget {
    fn parser_nesting_limit(&self) -> usize {
        self.depth_limit() + 1
    }
    fn scanner_refusal(&self, error: &ScanError) -> Option<DecodeLimitExceeded> {
        matches!(error.kind(), ErrorKind::RecursionLimitExceeded).then(|| self.depth_refusal())
    }
}

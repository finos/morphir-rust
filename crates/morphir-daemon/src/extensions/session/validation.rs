//! Validation of initialization data.
//!
//! Method results are checked by `morphir_host_native::validate_result`.

use super::controller::NegotiatedSession;
use super::transport::ExpectedExtension;
use crate::extensions::protocol::InitializeResult;
use crate::{DaemonError, Result};

pub(in crate::extensions) fn validate_negotiation(
    expected: ExpectedExtension,
    offered_versions: &[String],
    result: InitializeResult,
) -> Result<NegotiatedSession> {
    morphir_host::validate_negotiation(expected, offered_versions, result).map_err(Into::into)
}

/// The daemon's negotiation rules, run by the portable session core.
#[derive(Debug)]
pub(super) struct DaemonChecks {
    expected: ExpectedExtension,
}

impl DaemonChecks {
    pub(super) fn new(expected: ExpectedExtension) -> Self {
        Self { expected }
    }
}

impl morphir_host::SessionChecks for DaemonChecks {
    type Error = DaemonError;

    fn negotiate(
        &mut self,
        offered: &[String],
        result: InitializeResult,
    ) -> Result<NegotiatedSession> {
        validate_negotiation(self.expected.clone(), offered, result)
    }
}

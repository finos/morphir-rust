use thiserror::Error;

use crate::AvroDiagnostic;

/// An implementation invariant failed inside the Avro backend.
///
/// These failures are distinct from invalid options, unsupported Morphir
/// forms, and other user-facing diagnostics. The MEP boundary reports them as
/// extension execution failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("internal Avro backend invariant failed: {message}")]
pub struct AvroInternalError {
    message: String,
}

impl AvroInternalError {
    pub(crate) fn invariant(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// A projection or rendering failure before conversion to the MEP result.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AvroGenerationError {
    /// User-facing backend diagnostics.
    #[error("Avro generation produced {} diagnostic(s)", .0.len())]
    Diagnostics(Vec<AvroDiagnostic>),
    /// A backend implementation invariant failed.
    #[error(transparent)]
    Internal(#[from] AvroInternalError),
}

impl AvroGenerationError {
    /// Returns the user-facing diagnostics when this is a domain failure.
    #[must_use]
    pub fn as_diagnostics(&self) -> Option<&[AvroDiagnostic]> {
        match self {
            Self::Diagnostics(diagnostics) => Some(diagnostics),
            Self::Internal(_) => None,
        }
    }

    /// Consumes the failure, separating domain diagnostics from an internal error.
    pub fn into_diagnostics(self) -> Result<Vec<AvroDiagnostic>, AvroInternalError> {
        match self {
            Self::Diagnostics(diagnostics) => Ok(diagnostics),
            Self::Internal(error) => Err(error),
        }
    }
}

impl From<AvroDiagnostic> for AvroGenerationError {
    fn from(value: AvroDiagnostic) -> Self {
        Self::Diagnostics(vec![value])
    }
}

impl From<Vec<AvroDiagnostic>> for AvroGenerationError {
    fn from(value: Vec<AvroDiagnostic>) -> Self {
        Self::Diagnostics(value)
    }
}

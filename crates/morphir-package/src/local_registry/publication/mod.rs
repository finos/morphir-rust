//! Authenticated local publication with an explicitly qualified filesystem provider.
#[cfg(target_os = "macos")]
mod filesystem;
#[cfg(target_os = "macos")]
mod registry;
#[cfg(target_os = "macos")]
mod view;
#[cfg(target_os = "macos")]
pub use registry::Registry;
mod proposal;
#[cfg(target_os = "macos")]
mod verify;
pub use proposal::{Draft, Proposal};
use serde::{Deserialize, Serialize};

/// Exact current timestamp expected by a caller, never a snapshot digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Predecessor {
    /// Explicitly initialized registry with no committed timestamp.
    Absent,
    /// SHA256 of the complete current timestamp envelope bytes.
    Timestamp {
        /// Exact timestamp digest.
        digest: String,
    },
}
/// Publication result after all required flushes, or a verified no-op.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Complete successor durably committed.
    Committed,
    /// Exact release already present; status and metadata unchanged.
    Idempotent,
}
/// Exact timestamp associated with a successful result.
#[derive(Debug, Clone, Serialize)]
pub struct PublicationResult {
    /// Whether bytes were committed or already present.
    pub outcome: Outcome,
    /// SHA256 of the complete current timestamp.
    pub timestamp: String,
}
/// Publication failure; uncertain commit outcomes must never be retried blindly.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Malformed or inconsistent input.
    #[error("invalid-input: {0}")]
    Invalid(&'static str),
    /// Existing immutable release content differs.
    #[error("release-conflict")]
    ReleaseConflict,
    /// Same content with different immutable record or envelope.
    #[error("record-replacement-unsupported")]
    RecordReplacement,
    /// Current timestamp differs from the caller's base.
    #[error("publication-conflict: observed {observed:?}")]
    Conflict {
        /// Exact state required by the caller.
        expected: Predecessor,
        /// Exact observed current state.
        observed: Predecessor,
    },
    /// Proposed role version is at or below its durable floor.
    #[error("metadata-rollback: {role} received {received}, floor {floor}")]
    Rollback {
        /// Metadata role.
        role: &'static str,
        /// Highest occupied, reserved or committed version.
        floor: String,
        /// Proposed version.
        received: String,
    },
    /// Existing signature/profile checks rejected input.
    #[error(transparent)]
    Authentication(#[from] super::tuf::AdmissionError),
    /// Existing package authorization or decoding rejected input.
    #[error(transparent)]
    Diagnostic(#[from] super::Diagnostic),
    /// An anchored I/O operation failed before the commit point.
    #[error("io-failure: {0}")]
    Io(#[from] std::io::Error),
    /// Current timestamp changed, but durability could not be established.
    #[error("commit-outcome-uncertain")]
    CommitOutcomeUncertain,
}
impl Error {
    /// Normative structured evidence for supported publication failures.
    pub fn diagnostic(&self) -> Option<super::Diagnostic> {
        use super::{Category, Code, Diagnostic, Phase, Witness};
        match self {
            Self::Diagnostic(diagnostic) => Some(diagnostic.clone()),
            Self::Conflict { expected, observed } => Some(Diagnostic {
                category: Category::DomainRejection,
                code: Code::PublicationConflict,
                phase: Phase::Publication,
                witnesses: vec![Witness::Revision {
                    expected: expected.clone(),
                    actual: observed.clone(),
                }],
            }),
            _ => None,
        }
    }
}
fn digest(bytes: &[u8]) -> String {
    crate::digest::Digest::of_bytes(bytes).to_string()
}

#[cfg(all(test, target_os = "macos"))]
mod crash_tests;

#[cfg(all(test, target_os = "macos"))]
#[derive(Clone, Copy, PartialEq, Eq)]
enum FaultPoint {
    BeforeObjectPromotion,
    Reserved,
    BeforeTimestamp,
    AfterTimestamp,
    BeforeFinalFlush,
}
#[cfg(all(test, target_os = "macos"))]
#[derive(Clone, Copy)]
enum FaultAction {
    Crash,
    Fail,
}
#[cfg(all(test, target_os = "macos"))]
thread_local! {static FAULT:std::cell::Cell<Option<(FaultPoint,FaultAction)>>=const{std::cell::Cell::new(None)};}
#[cfg(all(test, target_os = "macos"))]
fn checkpoint(point: FaultPoint) -> Result<(), Error> {
    FAULT.with(|fault| match fault.get() {
        Some((expected, action)) if expected == point => match action {
            FaultAction::Crash => std::process::exit(73),
            FaultAction::Fail => Err(Error::CommitOutcomeUncertain),
        },
        _ => Ok(()),
    })
}

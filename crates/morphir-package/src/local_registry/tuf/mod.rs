//! Package TUF profile admission over exact signed evidence.
//!
//! These checks complement the upstream TUF update workflow. Successful profile,
//! quorum or link verification alone is not fresh repository or package authority.
mod admission;
mod budget;
mod context;
mod host;
mod links;
mod load;
mod profile;
mod quorum;
mod roots;
mod transport;

use super::Diagnostic;
pub use admission::ProfileAdmission;
pub use budget::AcquisitionBudget;
pub use host::{
    AdmissionBackend, CandidateEvidence, OperationBinding, OperationId, OperationSnapshot,
};
pub use links::verify_link;
pub use load::MetadataLoadError;
pub use profile::{ProfileMetadata, decode_profile};
pub use quorum::verify_quorum;
pub use roots::AuthenticatedRoots;
pub use transport::{EvidenceRecorder, ProfileTransport, profile_limits};

/// A failed package-profile or authenticated-context admission.
#[derive(Debug, thiserror::Error)]
pub enum AdmissionError {
    /// Strict JSON syntax rejected the evidence before authentication.
    #[error("invalid TUF JSON: {0:?}")]
    Syntax(Diagnostic),
    /// The metadata does not implement the supported package profile.
    #[error("unsupported or malformed TUF profile: {0}")]
    Profile(&'static str),
    /// An inclusive profile resource bound was exceeded.
    #[error("TUF resource limit: {0}")]
    Limit(&'static str),
    /// Too few distinct authorized raw public keys verified.
    #[error("TUF signature quorum was not met")]
    Signature,
    /// Evidence is not anchored in the independently provisioned forward chain.
    #[error("TUF root continuity was not established")]
    Continuity,
    /// Exact metadata bytes do not match their signed link.
    #[error("TUF metadata link mismatch: {0}")]
    Link(&'static str),
    /// The protected operation context or predecessor no longer matches.
    #[error("TUF protected operation context mismatch")]
    Context,
    /// Protected backend I/O failed; visible rows may have uncertain durability.
    #[error("TUF protected backend: {0}")]
    Backend(Box<dyn std::error::Error + Send + Sync>),
}

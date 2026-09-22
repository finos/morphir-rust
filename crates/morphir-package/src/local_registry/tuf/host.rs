use super::AdmissionError;
use crate::local_registry::{Digest, TufRole};
use async_trait::async_trait;
use package_tough::experimental_storage::{Revision, Snapshot, Transition};
use std::fmt;

/// Backend-generated identity of one durable candidate operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationId([u8; 32]);
impl OperationId {
    /// Wrap a backend-generated unique identifier; never derive it from registry input.
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}
/// Immutable binding recorded in the operation's durable marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationBinding {
    /// Distinguishes this operation from previous or concurrent attempts.
    pub id: OperationId,
    /// Independently provisioned repository identity.
    pub repository: Digest,
    /// Exact root digest when this operation began.
    pub initial_root: Digest,
    /// Authority revision when the marker was admitted.
    pub predecessor: Revision,
    /// Trusted fixed time for all checks in this operation.
    pub fixed_time: jiff::Timestamp,
}
/// Original acquired bytes retained with the operation's marker.
#[derive(Debug, Clone)]
pub struct CandidateEvidence {
    /// Logical top-level role, never a caller-controlled path.
    pub role: TufRole,
    /// Complete original envelope bytes.
    pub bytes: Vec<u8>,
}
/// One consistent protected-store read, including mandatory marker context.
#[derive(Debug, Clone)]
pub struct OperationSnapshot {
    /// Immutable, durably established operation marker binding.
    pub binding: OperationBinding,
    /// Authority state read in the same protected transaction.
    pub state: Snapshot,
    /// Exact bootstrap-to-current forward root chain.
    pub root_chain: Vec<Vec<u8>>,
    /// Exact candidate evidence durably retained for this marker.
    pub evidence: Vec<CandidateEvidence>,
    /// Bytes already charged to this same operation for distinct policy, lock,
    /// publisher, historical or other-repository metadata not listed above.
    /// The backend derives this from its operation inventory, never from a caller
    /// estimate. Logical keys are repository identity, role/path and exact digest.
    pub other_metadata_bytes: usize,
}
/// Required protected backend for package admission. There is no default provider.
///
/// Implementations must hold serialized process access for the lifetime of their
/// `ProfileAdmission`, read all fields atomically, and reject uninitialized,
/// uncertain or unreconciled stores. A recovered database row is not proof of a
/// successfully completed durability boundary. The production provider/recovery
/// implementation is separate work; the test backend is intentionally test-only.
#[async_trait]
pub trait AdmissionBackend: fmt::Debug + Send + Sync {
    /// Read existing state without implicit initialization or marker creation.
    /// Apply resource limits while reading BLOBs, before allocating unbounded input.
    async fn read(&self) -> Result<OperationSnapshot, AdmissionError>;
    /// Durably append exact evidence to the same marker iff authority revision
    /// still equals `expected`. This does not advance the authority revision.
    /// On any I/O error fail the operation; uncertain rows cannot be reused.
    /// Atomically enforce the shared 256 MiB metadata budget, including the other
    /// operation inputs reported by `other_metadata_bytes`; retries count once.
    async fn record(
        &self,
        expected: Revision,
        evidence: CandidateEvidence,
    ) -> Result<(), AdmissionError>;
    /// Atomically apply an admitted transition iff `expected` still matches and
    /// return its new revision. Retain root continuity/reset context atomically.
    /// Never change provisioning, marker binding, accepted time or grants.
    async fn commit(
        &self,
        expected: Revision,
        transition: &Transition,
    ) -> Result<Revision, AdmissionError>;
}

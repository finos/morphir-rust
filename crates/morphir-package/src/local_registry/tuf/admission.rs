use super::{
    AdmissionBackend, AdmissionError, AuthenticatedRoots, CandidateEvidence, EvidenceRecorder,
    OperationBinding, OperationSnapshot, decode_profile, verify_link, verify_quorum,
};
use crate::local_registry::{PolicyRepository, TufRole};
use async_trait::async_trait;
use package_tough::experimental_storage::{
    self as port, Admission, MetadataRole, RetainedMetadata, Revision, Snapshot, Storage,
    Transition,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Profile guard shared by bounded acquisition, Tough storage and transition admission.
///
/// Always pass the same instance as both Tough's `Storage` and `Admission`, and
/// as the evidence recorder of `ProfileTransport`. Its protected backend is a
/// required host dependency, not a permissive callback. It issues no grants.
#[derive(Debug)]
pub struct ProfileAdmission {
    policy: PolicyRepository,
    binding: OperationBinding,
    backend: Arc<dyn AdmissionBackend>,
    failed: AtomicBool,
}
impl ProfileAdmission {
    pub(super) fn fixed_time(&self) -> jiff::Timestamp {
        self.binding.fixed_time
    }

    /// Bind admission to independently trusted configuration and one existing
    /// durable operation marker. Every use revalidates the protected context.
    pub fn new(
        policy: PolicyRepository,
        binding: OperationBinding,
        backend: Arc<dyn AdmissionBackend>,
    ) -> Result<Self, AdmissionError> {
        if policy.identity() != &binding.repository {
            return Err(AdmissionError::Context);
        }
        Ok(Self {
            policy,
            binding,
            backend,
            failed: AtomicBool::new(false),
        })
    }
    fn backend_result<T>(&self, result: Result<T, AdmissionError>) -> Result<T, AdmissionError> {
        if result.is_err() {
            self.failed.store(true, Ordering::SeqCst);
        }
        result
    }
    async fn checked(&self) -> Result<(OperationSnapshot, AuthenticatedRoots), AdmissionError> {
        if self.failed.load(Ordering::SeqCst) {
            return Err(AdmissionError::Context);
        }
        let snapshot = self.backend_result(self.backend.read().await)?;
        if snapshot.binding != self.binding
            || snapshot.state.revision.0 < self.binding.predecessor.0
        {
            return Err(AdmissionError::Context);
        }
        let roots = super::context::check(&self.policy, &snapshot)?;
        Ok((snapshot, roots))
    }
    async fn predecessor(
        &self,
        expected: &Snapshot,
    ) -> Result<(OperationSnapshot, AuthenticatedRoots), AdmissionError> {
        let (snapshot, roots) = self.checked().await?;
        if !super::context::same_state(&snapshot.state, expected) {
            return Err(AdmissionError::Context);
        }
        Ok((snapshot, roots))
    }
    async fn admit(
        &self,
        expected: &Snapshot,
        transition: &Transition,
    ) -> Result<(), AdmissionError> {
        let (snapshot, mut roots) = self.predecessor(expected).await?;
        match transition {
            Transition::AdvanceRoot { root, baseline } => {
                super::context::require_evidence(&snapshot, TufRole::Root, root)?;
                let expected_baseline = expected
                    .reset_baseline
                    .as_ref()
                    .unwrap_or(&expected.current_root);
                if baseline != expected_baseline || super::context::rotation_count(&snapshot)? >= 32
                {
                    return Err(AdmissionError::Context);
                }
                roots.append(root)?;
            }
            Transition::FinishRootCycle { .. } => {} // Upstream determines the atomic floor reset.
            Transition::Retain { role, metadata } => {
                self.candidate(&snapshot, role, metadata)?;
                let parent = match role {
                    MetadataRole::Snapshot => Some((MetadataRole::Timestamp, TufRole::Timestamp)),
                    MetadataRole::Targets => Some((MetadataRole::Snapshot, TufRole::Snapshot)),
                    _ => None,
                };
                if let Some((name, parent_role)) = parent {
                    let parent = snapshot
                        .state
                        .metadata
                        .get(&name)
                        .ok_or(AdmissionError::Link("missing parent"))?;
                    // Current candidates cannot inherit a parent signed under a stale root.
                    if parent.acceptance_root != snapshot.state.current_root {
                        return Err(AdmissionError::Continuity);
                    }
                    verify_link(
                        &decode_profile(&parent.bytes, parent_role)?,
                        &decode_profile(&metadata.bytes, super::context::role_of(role)?)?,
                    )?;
                }
            }
        }
        Ok(())
    }
    fn candidate(
        &self,
        snapshot: &OperationSnapshot,
        role: &MetadataRole,
        metadata: &RetainedMetadata,
    ) -> Result<(), AdmissionError> {
        let role = super::context::role_of(role)?;
        super::context::require_evidence(snapshot, role, &metadata.bytes)?;
        if metadata.acceptance_root != snapshot.state.current_root {
            return Err(AdmissionError::Continuity);
        }
        verify_quorum(
            &decode_profile(&snapshot.state.current_root, TufRole::Root)?,
            &decode_profile(&metadata.bytes, role)?,
        )?;
        Ok(())
    }
}
fn port_error(error: AdmissionError) -> port::Error {
    port::Error::Backend(Box::new(error))
}
#[async_trait]
impl Storage for ProfileAdmission {
    async fn snapshot(&self) -> port::Result<Snapshot> {
        // Strict decoding and anchored retained authentication happen BEFORE the
        // upstream Session's own typed decode/crypto, not in its later callback.
        self.checked()
            .await
            .map(|(snapshot, _)| snapshot.state)
            .map_err(port_error)
    }
    async fn commit(&self, expected: Revision, transition: &Transition) -> port::Result<Revision> {
        let (snapshot, _) = self.checked().await.map_err(port_error)?;
        if snapshot.state.revision != expected {
            return Err(port::Error::Conflict);
        }
        self.admit(&snapshot.state, transition)
            .await
            .map_err(port_error)?;
        self.backend_result(self.backend.commit(expected, transition).await)
            .map_err(port_error)
    }
}
#[async_trait]
impl Admission for ProfileAdmission {
    async fn begin(&self, predecessor: &Snapshot, fixed_time: jiff::Timestamp) -> port::Result<()> {
        if fixed_time != self.binding.fixed_time {
            return Err(port::Error::Admission);
        }
        self.predecessor(predecessor)
            .await
            .map(|_| ())
            .map_err(port_error)
    }
    async fn transition(
        &self,
        predecessor: &Snapshot,
        transition: &Transition,
    ) -> port::Result<()> {
        self.admit(predecessor, transition)
            .await
            .map_err(port_error)
    }
    async fn timestamp_no_update(
        &self,
        predecessor: &Snapshot,
        candidate: &RetainedMetadata,
    ) -> port::Result<()> {
        let (snapshot, _) = self.predecessor(predecessor).await.map_err(port_error)?;
        // No link, expiry, authority write or grant is introduced at equality.
        self.candidate(&snapshot, &MetadataRole::Timestamp, candidate)
            .map_err(port_error)
    }
}
#[async_trait]
impl EvidenceRecorder for ProfileAdmission {
    async fn budget(&self, role: TufRole) -> Result<super::AcquisitionBudget, AdmissionError> {
        let (snapshot, _) = self.checked().await?;
        let remaining = 268_435_456 - super::context::budget(&snapshot)?;
        let mut previous = snapshot
            .evidence
            .iter()
            .filter(|e| e.role == role)
            .map(|e| e.bytes.clone())
            .collect::<Vec<_>>();
        if role == TufRole::Root {
            previous.extend(snapshot.root_chain);
        } else {
            previous.extend(
                snapshot
                    .state
                    .metadata
                    .iter()
                    .filter(|(name, _)| super::context::role_of(name).ok() == Some(role))
                    .map(|(_, metadata)| metadata.bytes.clone()),
            );
        }
        super::AcquisitionBudget::new(remaining, previous)
    }
    async fn record(&self, role: TufRole, bytes: &[u8]) -> Result<(), AdmissionError> {
        decode_profile(bytes, role)?;
        let (mut snapshot, _) = self.checked().await?;
        let evidence = CandidateEvidence {
            role,
            bytes: bytes.to_vec(),
        };
        snapshot.evidence.push(evidence.clone());
        super::context::budget(&snapshot)?;
        self.backend_result(self.backend.record(snapshot.state.revision, evidence).await)?;
        let (after, _) = self.checked().await?;
        if !super::context::same_state(&snapshot.state, &after.state) {
            return Err(AdmissionError::Context);
        }
        super::context::require_evidence(&after, role, bytes)
    }
}

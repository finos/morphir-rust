use super::{AdmissionError, AuthenticatedRoots, OperationSnapshot, decode_profile};
use crate::local_registry::{PolicyRepository, TufRole};
use package_tough::experimental_storage::{MetadataRole, Snapshot};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub(super) fn digest(bytes: &[u8]) -> String {
    format!(
        "sha256:{}",
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}
pub(super) fn role_of(role: &MetadataRole) -> Result<TufRole, AdmissionError> {
    match role {
        MetadataRole::Timestamp => Ok(TufRole::Timestamp),
        MetadataRole::Snapshot => Ok(TufRole::Snapshot),
        MetadataRole::Targets => Ok(TufRole::Targets),
        MetadataRole::Delegated(_) => Err(AdmissionError::Profile("delegated metadata")),
    }
}
pub(super) fn budget(snapshot: &OperationSnapshot) -> Result<usize, AdmissionError> {
    // Count unique original evidence, not repeated references to one retained root.
    let mut seen = HashSet::new();
    let mut total = snapshot.other_metadata_bytes;
    if total > 268_435_456 {
        return Err(AdmissionError::Limit("aggregate metadata"));
    }
    let bytes = snapshot
        .root_chain
        .iter()
        .map(Vec::as_slice)
        .chain(std::iter::once(snapshot.state.provisioned_root.as_slice()))
        .chain(std::iter::once(snapshot.state.current_root.as_slice()))
        .chain(snapshot.state.reset_baseline.iter().map(Vec::as_slice))
        .chain(
            snapshot
                .state
                .metadata
                .values()
                .flat_map(|m| [m.bytes.as_slice(), m.acceptance_root.as_slice()]),
        )
        .chain(snapshot.evidence.iter().map(|e| e.bytes.as_slice()));
    for bytes in bytes {
        if seen.insert(bytes) {
            total = total
                .checked_add(bytes.len())
                .ok_or(AdmissionError::Limit("aggregate metadata"))?;
            if total > 268_435_456 {
                return Err(AdmissionError::Limit("aggregate metadata"));
            }
        }
    }
    Ok(total)
}
pub(super) fn rotation_count(snapshot: &OperationSnapshot) -> Result<usize, AdmissionError> {
    let start = snapshot
        .root_chain
        .iter()
        .position(|root| digest(root) == snapshot.binding.initial_root.as_str())
        .ok_or(AdmissionError::Context)?;
    Ok(snapshot.root_chain.len() - start - 1)
}
pub(super) fn check(
    policy: &PolicyRepository,
    snapshot: &OperationSnapshot,
) -> Result<AuthenticatedRoots, AdmissionError> {
    budget(snapshot)?;
    if snapshot.root_chain.first() != Some(&snapshot.state.provisioned_root)
        || snapshot.root_chain.last() != Some(&snapshot.state.current_root)
    {
        return Err(AdmissionError::Continuity);
    }
    let roots = AuthenticatedRoots::from_policy(policy, &snapshot.root_chain)?;
    if rotation_count(snapshot)? > 32 {
        return Err(AdmissionError::Limit("root transitions"));
    }
    if snapshot
        .state
        .reset_baseline
        .as_ref()
        .is_some_and(|root| !roots.contains(root))
    {
        return Err(AdmissionError::Continuity);
    }
    if snapshot
        .state
        .accepted_time
        .is_some_and(|time| time > snapshot.binding.fixed_time)
    {
        return Err(AdmissionError::Context);
    }
    for (role, metadata) in &snapshot.state.metadata {
        let decoded = decode_profile(&metadata.bytes, role_of(role)?)?;
        roots.verify_retained(&metadata.acceptance_root, &decoded)?;
    }
    for evidence in &snapshot.evidence {
        decode_profile(&evidence.bytes, evidence.role)?;
    }
    Ok(roots)
}
pub(super) fn same_state(a: &Snapshot, b: &Snapshot) -> bool {
    a.revision == b.revision
        && a.provisioned_root == b.provisioned_root
        && a.current_root == b.current_root
        && a.reset_baseline == b.reset_baseline
        && a.accepted_time == b.accepted_time
        && a.metadata.len() == b.metadata.len()
        && a.metadata.iter().all(|(role, m)| {
            b.metadata.get(role).is_some_and(|other| {
                m.bytes == other.bytes && m.acceptance_root == other.acceptance_root
            })
        })
}
pub(super) fn require_evidence(
    snapshot: &OperationSnapshot,
    role: TufRole,
    bytes: &[u8],
) -> Result<(), AdmissionError> {
    if snapshot.evidence.iter().any(|e| {
        std::mem::discriminant(&e.role) == std::mem::discriminant(&role) && e.bytes == bytes
    }) {
        Ok(())
    } else {
        Err(AdmissionError::Context)
    }
}

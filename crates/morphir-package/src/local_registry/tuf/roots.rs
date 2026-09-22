use super::quorum::typed;
use super::{AdmissionError, ProfileMetadata, decode_profile, verify_quorum};
use crate::local_registry::{PolicyRepository, TufRole};
use package_tough::schema::{Root, Signed};
use sha2::{Digest, Sha256};

/// A forward root chain authenticated from an independently configured bootstrap.
///
/// Root expiry is intentionally left to the upstream current-update workflow:
/// historical continuity may include expired roots. This is not a fresh view.
#[derive(Debug, Clone)]
pub struct AuthenticatedRoots {
    roots: Vec<ProfileMetadata>,
}
impl AuthenticatedRoots {
    /// Verify the bootstrap version/digest, its self quorum, and each consecutive
    /// successor under both the old and new raw-key thresholds.
    ///
    /// A bootstrap above version one anchors only that root and its successors.
    /// The chain is bounded by the operation's aggregate metadata budget.
    pub fn from_policy(
        policy: &PolicyRepository,
        chain: &[Vec<u8>],
    ) -> Result<Self, AdmissionError> {
        let total = chain
            .iter()
            .try_fold(0usize, |n, root| n.checked_add(root.len()))
            .ok_or(AdmissionError::Limit("aggregate metadata"))?;
        if total > 268_435_456 {
            return Err(AdmissionError::Limit("aggregate metadata"));
        }
        let first = chain.first().ok_or(AdmissionError::Continuity)?;
        if format!(
            "sha256:{}",
            Sha256::digest(first)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ) != policy.bootstrap_root().digest().as_str()
        {
            return Err(AdmissionError::Continuity);
        }
        let first = decode_profile(first, TufRole::Root)?;
        let root: Signed<Root> = typed(&first)?;
        if root.signed.version.to_string() != policy.bootstrap_root().version().to_string() {
            return Err(AdmissionError::Continuity);
        }
        verify_quorum(&first, &first)?;
        let mut verified = Self { roots: vec![first] };
        for bytes in &chain[1..] {
            verified.append(bytes)?;
        }
        Ok(verified)
    }
    /// Exact evidence at the end of the verified chain.
    pub fn current(&self) -> &ProfileMetadata {
        self.roots.last().expect("nonempty verified chain")
    }
    /// Whether these exact bytes, including envelope whitespace, were anchored.
    pub fn contains(&self, bytes: &[u8]) -> bool {
        self.roots.iter().any(|root| root.bytes() == bytes)
    }
    /// Authenticate a retained role under an anchored acceptance root.
    /// This does not establish current expiry, a complete view, or a reusable grant.
    pub fn verify_retained(
        &self,
        acceptance_root: &[u8],
        role: &ProfileMetadata,
    ) -> Result<(), AdmissionError> {
        let root = self
            .roots
            .iter()
            .find(|root| root.bytes() == acceptance_root)
            .ok_or(AdmissionError::Continuity)?;
        verify_quorum(root, role)?;
        Ok(())
    }
    pub(crate) fn append(&mut self, bytes: &[u8]) -> Result<(), AdmissionError> {
        let next = decode_profile(bytes, TufRole::Root)?;
        let previous: Signed<Root> = typed(self.current())?;
        let candidate: Signed<Root> = typed(&next)?;
        if candidate.signed.version != previous.signed.version.successor() {
            return Err(AdmissionError::Continuity);
        }
        verify_quorum(self.current(), &next)?;
        verify_quorum(&next, &next)?;
        self.roots.push(next);
        Ok(())
    }
}

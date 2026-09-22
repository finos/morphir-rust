use super::{AdmissionError, ProfileMetadata};
use crate::local_registry::TufRole;
use package_tough::schema::key::Key;
use package_tough::schema::{Role, Root, Signed, Snapshot, Targets, Timestamp};
use serde::de::DeserializeOwned;
use std::{collections::HashSet, num::NonZeroU64};

pub(crate) fn typed<T: DeserializeOwned>(metadata: &ProfileMetadata) -> Result<T, AdmissionError> {
    serde_json::from_slice(metadata.bytes()).map_err(|_| AdmissionError::Profile("typed metadata"))
}

/// Verify a role's threshold by distinct authorized raw Ed25519 keys.
///
/// The supplied root must already be anchored by the caller. Each successful key
/// is checked by Tough over the unchanged signed object; key-ID aliases cannot
/// increase the count. No expiry, rollback, link or package authorization is implied.
pub fn verify_quorum(
    authority: &ProfileMetadata,
    candidate: &ProfileMetadata,
) -> Result<usize, AdmissionError> {
    if !matches!(authority.role(), TufRole::Root) {
        return Err(AdmissionError::Profile("authority must be root"));
    }
    let root: Signed<Root> = typed(authority)?;
    match candidate.role() {
        TufRole::Root => verify::<Root>(&root.signed, candidate),
        TufRole::Timestamp => verify::<Timestamp>(&root.signed, candidate),
        TufRole::Snapshot => verify::<Snapshot>(&root.signed, candidate),
        TufRole::Targets => verify::<Targets>(&root.signed, candidate),
    }
}
fn verify<T: Role + DeserializeOwned + Clone>(
    root: &Root,
    candidate: &ProfileMetadata,
) -> Result<usize, AdmissionError> {
    let signed: Signed<T> = typed(candidate)?;
    let authority = root.roles.get(&T::TYPE).ok_or(AdmissionError::Signature)?;
    // Preserve the upstream envelope rule even though each crypto call projects
    // one signature. In particular, duplicate signature IDs are not ignored.
    let mut ids = HashSet::new();
    if signed
        .signatures
        .iter()
        .any(|signature| !ids.insert(signature.keyid.clone()))
    {
        return Err(AdmissionError::Signature);
    }
    let mut verified = HashSet::new();
    for signature in &signed.signatures {
        if !authority.keyids.contains(&signature.keyid) {
            continue;
        }
        let Some(Key::Ed25519 { keyval, .. }) = root.keys.get(&signature.keyid) else {
            continue;
        };
        let raw: [u8; 32] = keyval
            .public
            .as_ref()
            .try_into()
            .map_err(|_| AdmissionError::Profile("Ed25519 key width"))?;
        let mut projection = root.clone();
        let role = projection
            .roles
            .get_mut(&T::TYPE)
            .expect("selected authority");
        role.keyids = vec![signature.keyid.clone()];
        role.threshold = NonZeroU64::new(1).expect("one is positive");
        let projected = Signed {
            signed: signed.signed.clone(),
            signatures: vec![signature.clone()],
        };
        if projection.verify_role(&projected).is_ok() {
            verified.insert(raw);
        }
    }
    if (verified.len() as u64) < authority.threshold.get() {
        return Err(AdmissionError::Signature);
    }
    Ok(verified.len())
}

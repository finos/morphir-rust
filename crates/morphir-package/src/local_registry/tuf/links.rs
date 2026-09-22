use super::{AdmissionError, ProfileMetadata};
use crate::local_registry::TufRole;
use sha2::{Digest, Sha256, Sha512};

/// Check the signed parent link against the child's original envelope bytes.
///
/// Requires exact version, byte length, SHA256 and every advertised supported
/// hash (currently SHA512). Both envelopes must separately pass authentication.
pub fn verify_link(
    parent: &ProfileMetadata,
    child: &ProfileMetadata,
) -> Result<(), AdmissionError> {
    let name = match (parent.role(), child.role()) {
        (TufRole::Timestamp, TufRole::Snapshot) => "snapshot.json",
        (TufRole::Snapshot, TufRole::Targets) => "targets.json",
        _ => return Err(AdmissionError::Link("role pair")),
    };
    let link = &parent.document["signed"]["meta"][name];
    if link["version"] != child.document["signed"]["version"] {
        return Err(AdmissionError::Link("version"));
    }
    if link["length"].as_u64() != Some(child.bytes().len() as u64) {
        return Err(AdmissionError::Link("length"));
    }
    if !matches_hash(&link["hashes"]["sha256"], &Sha256::digest(child.bytes())) {
        return Err(AdmissionError::Link("sha256"));
    }
    if let Some(hash) = link["hashes"].get("sha512")
        && !matches_hash(hash, &Sha512::digest(child.bytes()))
    {
        return Err(AdmissionError::Link("sha512"));
    }
    Ok(())
}
fn matches_hash(value: &serde_json::Value, actual: &[u8]) -> bool {
    value.as_str().is_some_and(|expected| {
        expected.len() == actual.len() * 2
            && expected
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .zip(actual)
                .all(|(pair, byte)| {
                    let digit = |b: u8| (b as char).to_digit(16);
                    digit(pair[0])
                        .zip(digit(pair[1]))
                        .is_some_and(|(a, b)| (a * 16 + b) as u8 == *byte)
                })
    })
}

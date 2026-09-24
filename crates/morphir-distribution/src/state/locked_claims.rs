//! Claim pins in the lock's extensible index metadata.

use super::*;
use serde_json::Value;

/// Released beta.5/6/7 lock readers reject unknown top-level fields but ignore
/// unknown index members. Keep the pin here so those readers can still activate
/// new installs. Ordinary index provenance and version-1 lock bytes stay intact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct LockedIndex {
    #[serde(flatten)]
    pub provenance: IndexProvenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claims: Option<Value>,
}

impl LockedIndex {
    pub fn validate_claims(&self, installed: &InstalledExtension) -> Result<()> {
        // Released CLIs wrote v2 installs without this pin. Preserve their
        // activation behavior, including claim-only flags, until reinstall.
        let Some(pinned) = &self.claims else {
            return Ok(());
        };
        let supplied = installed.claims.supplied_value();
        let mismatch = match supplied {
            Some(supplied) => differing_member(pinned, supplied, "claims"),
            None => Some("claims".to_owned()),
        };
        if let Some(member) = mismatch {
            return Err(DistributionError::InstalledClaimsMismatch {
                id: installed.extension_id.clone(),
                member,
            });
        }
        Ok(())
    }
}

/// Compare preserved JSON, including members unknown to this host. Object order
/// is irrelevant; missing members, array order, and explicit false remain distinct.
fn differing_member(pinned: &Value, supplied: &Value, path: &str) -> Option<String> {
    if pinned == supplied {
        return None;
    }
    if let (Value::Object(pinned), Value::Object(supplied)) = (pinned, supplied) {
        for key in pinned.keys().chain(supplied.keys()) {
            let member = format!("{path}.{key}");
            match (pinned.get(key), supplied.get(key)) {
                (Some(left), Some(right)) => {
                    if let Some(mismatch) = differing_member(left, right, &member) {
                        return Some(mismatch);
                    }
                }
                _ => return Some(member),
            }
        }
    }
    Some(path.to_owned())
}

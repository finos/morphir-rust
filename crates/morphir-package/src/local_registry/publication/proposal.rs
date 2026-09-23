use super::{Error, Predecessor, digest};
use crate::authoring::LocalSigningKey;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Exact signed successor metadata supplied to publication, without signing keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    /// Exact signed targets envelope.
    pub targets: Vec<u8>,
    /// Exact signed snapshot envelope.
    pub snapshot: Vec<u8>,
    /// Exact signed timestamp envelope.
    pub timestamp: Vec<u8>,
}
/// An unreserved successor description. Another writer can invalidate its base.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub(super) predecessor: Predecessor,
    pub(super) targets: Value,
    pub(super) snapshot_version: package_tough::schema::Version,
    pub(super) timestamp_version: package_tough::schema::Version,
}
impl Draft {
    /// The exact current timestamp observed while preparing this proposal.
    pub fn predecessor(&self) -> &Predecessor {
        &self.predecessor
    }
    /// Sign the complete successor outside the key-free publication operation.
    pub fn sign(
        &self,
        targets_key: &LocalSigningKey,
        snapshot_key: &LocalSigningKey,
        timestamp_key: &LocalSigningKey,
    ) -> Result<Proposal, Error> {
        let sign = |key: &LocalSigningKey, value: &Value| {
            key.sign_tuf(value)
                .map_err(|_| Error::Invalid("signing input"))
        };
        let targets = sign(targets_key, &self.targets)?;
        let mut link = pin(&targets);
        link["version"] = self.targets["version"].clone();
        let snapshot = sign(
            snapshot_key,
            &json!({"_type":"snapshot","spec_version":"1.0.36","version":self.snapshot_version,
            "expires":self.targets["expires"],"meta":{"targets.json":link}}),
        )?;
        let mut link = pin(&snapshot);
        link["version"] = json!(self.snapshot_version);
        let timestamp = sign(
            timestamp_key,
            &json!({"_type":"timestamp","spec_version":"1.0.36","version":self.timestamp_version,
            "expires":self.targets["expires"],"meta":{"snapshot.json":link}}),
        )?;
        Ok(Proposal {
            targets,
            snapshot,
            timestamp,
        })
    }
}
fn pin(bytes: &[u8]) -> Value {
    json!({"length": bytes.len(), "hashes": {"sha256": &digest(bytes)[7..]}})
}

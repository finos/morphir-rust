use super::{AuthoredLibrary, Error};
use crate::{digest::Digest, local_registry::PUBLISHER_PAYLOAD_TYPE};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_zebra::SigningKey;
use serde_json::json;

/// An explicitly supplied Ed25519 key. It cannot be serialized or debug-printed.
///
/// This adapter performs signing only; possession does not establish authorization.
pub struct LocalSigningKey(SigningKey);
impl LocalSigningKey {
    /// Load a caller-supplied 32-byte Ed25519 seed. No keys are generated implicitly.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self(SigningKey::from(seed))
    }
    /// Raw public key in the draft publisher policy's lowercase hex format.
    pub fn public_key_hex(&self) -> String {
        let bytes: [u8; 32] = self.0.verification_key().into();
        hex(&bytes)
    }
    pub(crate) fn signature(&self, bytes: &[u8]) -> [u8; 64] {
        self.0.sign(bytes).into()
    }
}
/// Exact public release artifacts. There is no signing key in this value.
#[derive(Debug, Clone)]
pub struct SignedLibrary {
    record: Vec<u8>,
    envelope: Vec<u8>,
}
impl SignedLibrary {
    /// Canonical registry record with one final line feed.
    pub fn record_bytes(&self) -> &[u8] {
        &self.record
    }
    /// DSSE envelope binding the exact canonical release statement.
    pub fn envelope_bytes(&self) -> &[u8] {
        &self.envelope
    }
}
impl AuthoredLibrary {
    /// Sign the verified Library identity without granting it publication authority.
    pub fn sign(&self, key: &LocalSigningKey) -> Result<SignedLibrary, Error> {
        let manifest = self.metadata().value();
        let mut statement = json!({"formatVersion":"0.1.0-draft.3","kind":"LibraryReleaseStatement",
            "release":{"packagePath":manifest["packagePath"],"version":manifest["version"]},
            "irPackageName":manifest["ir"]["packageName"],"dependencies":[],
            "manifestDigest":self.metadata().manifest_digest().to_string(),
            "contentDigest":self.metadata().content_digest().to_string()});
        let payload = crate::metadata::NormalizedMetadata::parse(&statement.to_string())
            .map_err(|_| Error::InvalidInput)?
            .as_str()
            .as_bytes()
            .to_vec();
        let mut pae = format!(
            "DSSEv1 {} {} {} ",
            PUBLISHER_PAYLOAD_TYPE.len(),
            PUBLISHER_PAYLOAD_TYPE,
            payload.len()
        )
        .into_bytes();
        pae.extend_from_slice(&payload);
        let envelope = serde_json::to_vec(&json!({"payloadType":PUBLISHER_PAYLOAD_TYPE,
            "payload":STANDARD.encode(&payload),"signatures":[{"keyid":key.public_key_hex(),
            "sig":STANDARD.encode(key.signature(&pae))}]}))
        .map_err(|_| Error::InvalidInput)?;
        let digest = self.metadata().content_digest().to_string();
        statement["kind"] = json!("LibraryRegistryRecord");
        statement["source"] =
            json!({"kind":"registry-directory","path":format!("bundles/{}",&digest[7..])});
        statement["statement"] = json!({"path":format!("statements/{}.json",&digest[7..]),
            "digest":Digest::of_bytes(&envelope).to_string()});
        let mut record = crate::metadata::NormalizedMetadata::parse(&statement.to_string())
            .map_err(|_| Error::InvalidInput)?
            .as_str()
            .as_bytes()
            .to_vec();
        record.push(b'\n');
        Ok(SignedLibrary { record, envelope })
    }
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl LocalSigningKey {
    /// Public TUF Ed25519 key object, with no authority implied.
    pub fn tuf_public_key(&self) -> serde_json::Value {
        json!({"keytype":"ed25519","scheme":"ed25519","keyval":{"public":self.public_key_hex()}})
    }
    /// TUF key identifier calculated by the existing TUF implementation.
    pub fn tuf_key_id(&self) -> Result<String, Error> {
        let key: package_tough::schema::key::Key =
            serde_json::from_value(self.tuf_public_key()).map_err(|_| Error::InvalidInput)?;
        Ok(hex(key.key_id().map_err(|_| Error::InvalidInput)?.as_ref()))
    }
    /// Sign a complete top-level role using Tough's canonical serialization.
    /// The publication engine separately authenticates all signatures and links.
    pub fn sign_tuf(&self, signed: &serde_json::Value) -> Result<Vec<u8>, Error> {
        use crate::local_registry::{TufRole, tuf::decode_profile};
        use package_tough::schema::{Root, Snapshot, Targets, Timestamp};
        let (canonical, role) = match signed["_type"].as_str() {
            Some("root") => (role_bytes::<Root>(signed)?, TufRole::Root),
            Some("timestamp") => (role_bytes::<Timestamp>(signed)?, TufRole::Timestamp),
            Some("snapshot") => (role_bytes::<Snapshot>(signed)?, TufRole::Snapshot),
            Some("targets") => (role_bytes::<Targets>(signed)?, TufRole::Targets),
            _ => return Err(Error::InvalidInput),
        };
        let canonical: serde_json::Value =
            serde_json::from_slice(&canonical).map_err(|_| Error::InvalidInput)?;
        let message = match role {
            TufRole::Root => role_bytes::<Root>(&canonical)?,
            TufRole::Timestamp => role_bytes::<Timestamp>(&canonical)?,
            TufRole::Snapshot => role_bytes::<Snapshot>(&canonical)?,
            TufRole::Targets => role_bytes::<Targets>(&canonical)?,
        };
        let output=serde_json::to_vec(&json!({"signed":canonical,"signatures":[{"keyid":self.tuf_key_id()?,"sig":hex(&self.signature(&message))}]})).map_err(|_|Error::InvalidInput)?;
        decode_profile(&output, role).map_err(|_| Error::InvalidInput)?;
        Ok(output)
    }
}
fn role_bytes<T: package_tough::schema::Role + serde::de::DeserializeOwned>(
    value: &serde_json::Value,
) -> Result<Vec<u8>, Error> {
    serde_json::from_value::<T>(value.clone())
        .map_err(|_| Error::InvalidInput)?
        .canonical_form()
        .map_err(|_| Error::InvalidInput)
}

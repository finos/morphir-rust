use super::AdmissionError;
use crate::local_registry::{JsonDomain, Phase, Subject, TufRole, decode_json_domain};
use serde_json::{Map, Value};

/// Structurally admitted metadata; it carries no authentication authority.
#[derive(Debug, Clone)]
pub struct ProfileMetadata {
    pub(crate) bytes: Vec<u8>,
    pub(crate) role: TufRole,
    pub(crate) document: serde_json::Value,
}

impl ProfileMetadata {
    /// The original complete envelope bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// The expected metadata role.
    pub fn role(&self) -> TufRole {
        self.role
    }
    /// The complete decoded document, including signed extensions.
    pub fn document(&self) -> &serde_json::Value {
        &self.document
    }
}

/// Decode bounded package TUF syntax and shape without authenticating it.
///
/// Exact bytes and signed extensions remain intact. Signature verification, expiry,
/// authenticated links, root continuity and operation-wide budgets are separate checks.
///
/// ```
/// use morphir_package::local_registry::{TufRole, tuf::decode_profile};
/// assert!(decode_profile(b"{}", TufRole::Root).is_err());
/// ```
pub fn decode_profile(bytes: &[u8], role: TufRole) -> Result<ProfileMetadata, AdmissionError> {
    let decoded = decode_json_domain(
        bytes,
        JsonDomain::Tuf(role),
        &Subject::Lock,
        Phase::Repository,
    )
    .map_err(AdmissionError::Syntax)?;
    // The syntax pass checks duplicate decoded keys and every integer token before
    // serde's exact-number representation is used to inspect typed fields.
    let document: Value = serde_json::from_str(decoded.text())
        .map_err(|_| AdmissionError::Profile("TUF JSON encoding"))?;
    let envelope = object(&document, "TUF envelope")?;
    let signed = object(field(envelope, "signed")?, "TUF signed object")?;
    let name = match role {
        TufRole::Root => "root",
        TufRole::Timestamp => "timestamp",
        TufRole::Snapshot => "snapshot",
        TufRole::Targets => "targets",
    };
    require(
        string(field(signed, "_type")?, "TUF role")? == name,
        "TUF role",
    )?;
    require(
        string(field(signed, "spec_version")?, "TUF specification")? == "1.0.36",
        "TUF specification",
    )?;
    integer(field(signed, "version")?, true)?;
    string(field(signed, "expires")?, "TUF expiry")?;
    signatures(field(envelope, "signatures")?)?;
    match role {
        TufRole::Root => root(signed)?,
        TufRole::Timestamp => parent_link(signed, "snapshot.json")?,
        TufRole::Snapshot => parent_link(signed, "targets.json")?,
        TufRole::Targets => targets(signed)?,
    }
    Ok(ProfileMetadata {
        bytes: bytes.to_vec(),
        role,
        document,
    })
}

fn root(signed: &Map<String, Value>) -> Result<(), AdmissionError> {
    require(
        field(signed, "consistent_snapshot")? == &Value::Bool(true),
        "consistent snapshots are required",
    )?;
    let keys = object(field(signed, "keys")?, "root keys")?;
    limit(keys.len(), 256, "tuf-keys")?;
    for (id, value) in keys {
        hex(id, Some(64))?;
        let key = object(value, "TUF key")?;
        require(
            string(field(key, "keytype")?, "TUF key type")? == "ed25519"
                && string(field(key, "scheme")?, "TUF key scheme")? == "ed25519",
            "Ed25519 keys and scheme are required",
        )?;
        let keyval = object(field(key, "keyval")?, "TUF key value")?;
        hex(
            string(field(keyval, "public")?, "TUF public key")?,
            Some(64),
        )?;
    }
    let roles = object(field(signed, "roles")?, "root roles")?;
    require(
        roles.len() == 4,
        "exactly four top-level roles are required",
    )?;
    for name in ["root", "timestamp", "snapshot", "targets"] {
        let role = object(field(roles, name)?, "role keys")?;
        let ids = array(field(role, "keyids")?, "role key IDs")?;
        limit(ids.len(), 64, "role-keys")?;
        for id in ids {
            hex(string(id, "role key ID")?, Some(64))?;
        }
        let threshold = integer(field(role, "threshold")?, true)?;
        let count = ids.len().to_string();
        require(
            threshold.len() < count.len() || (threshold.len() == count.len() && threshold <= count),
            "role threshold exceeds key count",
        )?;
    }
    Ok(())
}

fn signatures(value: &Value) -> Result<(), AdmissionError> {
    let signatures = array(value, "TUF signatures")?;
    limit(signatures.len(), 64, "signatures")?;
    for value in signatures {
        let signature = object(value, "TUF signature")?;
        hex(
            string(field(signature, "keyid")?, "signature key ID")?,
            None,
        )?;
        // Wrong-length but well-decoded signatures cannot contribute a quorum.
        // Signature validity belongs to the unchanged cryptographic verifier.
        hex(string(field(signature, "sig")?, "signature bytes")?, None)?;
    }
    Ok(())
}

fn parent_link(signed: &Map<String, Value>, name: &'static str) -> Result<(), AdmissionError> {
    let meta = object(field(signed, "meta")?, "parent metadata links")?;
    require(
        meta.len() == 1,
        "parent metadata must contain exactly one link",
    )?;
    let link = object(field(meta, name)?, "metadata link")?;
    integer(field(link, "version")?, true)?;
    file_description(link)
}

fn targets(signed: &Map<String, Value>) -> Result<(), AdmissionError> {
    require(
        !signed.contains_key("delegations"),
        "delegated targets are unsupported",
    )?;
    let targets = object(field(signed, "targets")?, "targets map")?;
    limit(targets.len(), 8192, "target-entries")?;
    for value in targets.values() {
        file_description(object(value, "target description")?)?;
    }
    Ok(())
}

fn file_description(description: &Map<String, Value>) -> Result<(), AdmissionError> {
    integer(field(description, "length")?, false)?;
    let hashes = object(field(description, "hashes")?, "file hashes")?;
    hex(
        string(field(hashes, "sha256")?, "SHA-256 digest")?,
        Some(64),
    )?;
    if let Some(hash) = hashes.get("sha512") {
        hex(string(hash, "SHA-512 digest")?, Some(128))?;
    }
    Ok(())
}

fn field<'a>(
    value: &'a Map<String, Value>,
    name: &'static str,
) -> Result<&'a Value, AdmissionError> {
    value.get(name).ok_or(AdmissionError::Profile(name))
}

fn object<'a>(
    value: &'a Value,
    name: &'static str,
) -> Result<&'a Map<String, Value>, AdmissionError> {
    value.as_object().ok_or(AdmissionError::Profile(name))
}

fn array<'a>(value: &'a Value, name: &'static str) -> Result<&'a [Value], AdmissionError> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or(AdmissionError::Profile(name))
}

fn string<'a>(value: &'a Value, name: &'static str) -> Result<&'a str, AdmissionError> {
    value.as_str().ok_or(AdmissionError::Profile(name))
}

fn integer(value: &Value, positive: bool) -> Result<String, AdmissionError> {
    let value = value
        .as_number()
        .ok_or(AdmissionError::Profile("TUF integer"))?
        .to_string();
    require(
        value.bytes().all(|b| b.is_ascii_digit())
            && (value == "0" || !value.starts_with('0'))
            && (!positive || value != "0"),
        "TUF integer domain",
    )?;
    Ok(value)
}

fn hex(value: &str, width: Option<usize>) -> Result<(), AdmissionError> {
    require(
        value.len().is_multiple_of(2)
            && width.is_none_or(|width| value.len() == width)
            && value.bytes().all(|b| b.is_ascii_hexdigit()),
        "TUF hexadecimal encoding",
    )
}

fn require(condition: bool, message: &'static str) -> Result<(), AdmissionError> {
    if condition {
        Ok(())
    } else {
        Err(AdmissionError::Profile(message))
    }
}

fn limit(observed: usize, maximum: usize, name: &'static str) -> Result<(), AdmissionError> {
    if observed <= maximum {
        Ok(())
    } else {
        Err(AdmissionError::Limit(name))
    }
}

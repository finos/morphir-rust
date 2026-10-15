use super::{MAX_REQUEST_BYTES, ServiceError};
use crate::resolution::{PackagePath, ReleaseId, StableVersion};
use ion_rs::{Element, Reader, ValueRef, v1_0::Binary};
use std::{collections::BTreeMap, path::PathBuf};

pub(super) struct Correlation {
    pub invocation: String,
    pub scope: String,
}
pub(super) struct Restore {
    pub correlation: Correlation,
    pub release: ReleaseId,
    pub registry: PathBuf,
    pub state: PathBuf,
    pub output: PathBuf,
    pub policy: Vec<u8>,
    pub lock: Vec<u8>,
}
pub(super) enum Request {
    Capabilities(Correlation),
    Restore(Box<Restore>),
}
fn invalid<T>() -> Result<T, ServiceError> {
    Err(ServiceError::InvalidRequest)
}
fn text(
    fields: &BTreeMap<String, Element>,
    key: &str,
    maximum: usize,
) -> Result<String, ServiceError> {
    let value = fields
        .get(key)
        .and_then(Element::as_string)
        .ok_or(ServiceError::InvalidRequest)?;
    if value.is_empty() || value.len() > maximum || value.contains('\0') {
        return invalid();
    }
    Ok(value.to_owned())
}
fn path(fields: &BTreeMap<String, Element>, key: &str) -> Result<PathBuf, ServiceError> {
    let path = PathBuf::from(text(fields, key, 4096)?);
    if !path.is_absolute() {
        return invalid();
    }
    Ok(path)
}
fn blob(fields: &BTreeMap<String, Element>, key: &str) -> Result<Vec<u8>, ServiceError> {
    let value = fields
        .get(key)
        .and_then(Element::as_blob)
        .ok_or(ServiceError::InvalidRequest)?;
    if value.is_empty() {
        return invalid();
    }
    Ok(value.to_vec())
}

pub(super) fn request(bytes: &[u8]) -> Result<Request, ServiceError> {
    if bytes.len() > MAX_REQUEST_BYTES || !bytes.starts_with(&[0xe0, 1, 0, 0xea]) {
        return Err(ServiceError::InvalidFrame);
    }
    let mut reader = Reader::new(Binary, bytes).map_err(|_| ServiceError::InvalidFrame)?;
    // The request grammar is flat. Read only allowed scalar fields through the
    // lazy reader; never materialize attacker-supplied nested containers.
    let (profile, fields) = {
        let value = reader
            .expect_next()
            .map_err(|_| ServiceError::InvalidFrame)?;
        let mut annotations = value.annotations();
        let annotation = annotations
            .next()
            .ok_or(ServiceError::InvalidFrame)?
            .map_err(|_| ServiceError::InvalidFrame)?;
        let profile = annotation
            .text()
            .ok_or(ServiceError::InvalidFrame)?
            .to_owned();
        if annotations.next().is_some() {
            return Err(ServiceError::InvalidFrame);
        }
        let allowed: &[&str] = match profile.as_str() {
            "morphir_provider_capabilities_request_v1" => &["invocation", "projectScope"],
            "morphir_provider_snapshot_request_v1" => &[
                "invocation",
                "projectScope",
                "release",
                "assurance",
                "registry",
                "state",
                "output",
                "policy",
                "lock",
            ],
            _ => return Err(ServiceError::InvalidFrame),
        };
        let structure = value
            .read()
            .map_err(|_| ServiceError::InvalidFrame)?
            .expect_struct()
            .map_err(|_| ServiceError::InvalidFrame)?;
        let mut fields = BTreeMap::new();
        for field in &structure {
            let field = field.map_err(|_| ServiceError::InvalidFrame)?;
            let symbol = field.name().map_err(|_| ServiceError::InvalidFrame)?;
            let name = symbol.text().ok_or(ServiceError::InvalidRequest)?;
            if !allowed.contains(&name) || fields.contains_key(name) {
                return invalid();
            }
            let value = field.value();
            if value.has_annotations() {
                return invalid();
            }
            let scalar = match value.read().map_err(|_| ServiceError::InvalidFrame)? {
                ValueRef::String(text) => Element::from(text.text()),
                ValueRef::Blob(bytes) => Element::from(ion_rs::Blob::from(bytes.to_vec())),
                _ => return invalid(),
            };
            fields.insert(name.to_owned(), scalar);
        }
        if fields.len() != allowed.len() {
            return invalid();
        }
        (profile, fields)
    };
    if reader
        .next()
        .map_err(|_| ServiceError::InvalidFrame)?
        .is_some()
    {
        return Err(ServiceError::InvalidFrame);
    }
    let invocation = text(&fields, "invocation", 128)?;
    if !invocation
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        return invalid();
    }
    let scope = text(&fields, "projectScope", 64)?;
    if scope.len() != 64
        || !scope
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return invalid();
    }
    let correlation = Correlation { invocation, scope };
    if profile == "morphir_provider_capabilities_request_v1" {
        return Ok(Request::Capabilities(correlation));
    }
    if text(&fields, "assurance", 32)? != "portable" {
        return invalid();
    }
    let release = text(&fields, "release", 1024)?;
    let (package, version) = release
        .split_once('@')
        .ok_or(ServiceError::InvalidRequest)?;
    let release = ReleaseId::new(
        PackagePath::parse(package).map_err(|_| ServiceError::InvalidRequest)?,
        StableVersion::parse(version).map_err(|_| ServiceError::InvalidRequest)?,
    );
    Ok(Request::Restore(Box::new(Restore {
        correlation,
        release,
        registry: path(&fields, "registry")?,
        state: path(&fields, "state")?,
        output: path(&fields, "output")?,
        policy: blob(&fields, "policy")?,
        lock: blob(&fields, "lock")?,
    })))
}

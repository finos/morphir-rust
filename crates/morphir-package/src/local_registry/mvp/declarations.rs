//! Pure validation of bounded package declarations in authenticated targets.
use super::{Error, require};
use crate::{
    local_registry::*,
    resolution::{PackagePath, ReleaseId, StableVersion},
};
use serde_json::{Value, json};
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Status {
    Active,
    Yanked,
    Revoked,
}
fn release(value: &Value) -> Result<ReleaseId, Error> {
    let object = value
        .as_object()
        .ok_or(Error::Refused("target custom release"))?;
    require(object.len() == 2, "target custom release")?;
    let package = object
        .get("packagePath")
        .and_then(Value::as_str)
        .ok_or(Error::Refused("target custom release"))?;
    let version = object
        .get("version")
        .and_then(Value::as_str)
        .ok_or(Error::Refused("target custom release"))?;
    Ok(ReleaseId::new(
        PackagePath::parse(package).map_err(|_| Error::Refused("target custom release"))?,
        StableVersion::parse(version).map_err(|_| Error::Refused("target custom release"))?,
    ))
}
pub(super) fn custom(target: &Value, kind: &str, fields: usize) -> Result<ReleaseId, Error> {
    let authority = target["custom"]["morphir"]
        .as_object()
        .ok_or(Error::Refused("target custom metadata"))?;
    require(
        authority.len() == fields
            && authority.get("formatVersion") == Some(&json!("0.1.0-draft.3"))
            && authority.get("kind") == Some(&json!(kind)),
        "target custom metadata",
    )?;
    release(
        authority
            .get("release")
            .ok_or(Error::Refused("target custom release"))?,
    )
}
pub(super) fn reference(path: &str, target: &Value) -> Result<ObjectReference, Error> {
    let path = RegistryPath::parse(path).map_err(|_| Error::Refused("unsafe target path"))?;
    let hash = target["hashes"]["sha256"]
        .as_str()
        .ok_or(Error::Refused("target hash"))?;
    let digest =
        Digest::parse(&format!("sha256:{hash}")).map_err(|_| Error::Refused("target hash"))?;
    Ok(ObjectReference { path, digest })
}

impl Status {
    pub(super) fn parse(target: &Value) -> Result<Self, Error> {
        match target["custom"]["morphir"]["status"].as_str() {
            Some("active") => Ok(Self::Active),
            Some("yanked") => Ok(Self::Yanked),
            Some("revoked") => Ok(Self::Revoked),
            _ => Err(Error::Refused("unsupported target custom status")),
        }
    }
}
/// Refresh observes declarations without opening release records or statements.
/// Revocation cannot be forgotten by a later active assertion in this MVP.
pub(super) fn check_refresh(targets: &Value) -> Result<(), Error> {
    let targets = targets.as_object().ok_or(Error::Refused("targets map"))?;
    let mut releases = 0;
    for (path, target) in targets {
        reference(path, target)?;
        if path.starts_with("statements/") {
            custom(target, "LibraryReleaseStatement", 3)?;
        } else {
            require(
                path.starts_with("records/"),
                "unsupported package target kind",
            )?;
            require(releases < 4096, "catalog release limit")?;
            releases += 1;
            custom(target, "LibraryRelease", 4)?;
            require(
                Status::parse(target)? != Status::Revoked,
                "revocation transition unsupported by MVP; manual intervention required",
            )?;
        }
    }
    Ok(())
}

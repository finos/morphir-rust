//! Author dependency-free classic V4 Libraries from exact compiled IR bytes.
//!
//! Authoring checks package integrity; it does not grant publisher authority.

mod sign;
pub use sign::{LocalSigningKey, SignedLibrary};

use crate::{
    digest::Digest,
    library::{LibraryInput, VerifiedLibrarySet},
    metadata::NormalizedMetadata,
    resolution::{PackagePath, StableVersion},
    schema::PackageSchemas,
    strict_json,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// A rejected authoring input. No partial artifact is returned.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The input exceeds the initial profile's bound.
    #[error("authoring input exceeds resource limit")]
    ResourceLimit,
    /// Authoring accepts only complete dependency-free classic V4 Libraries.
    #[error("invalid dependency-free classic V4 Library authoring input")]
    InvalidInput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Input {
    package_path: String,
    version: String,
    dependencies: BTreeMap<String, Value>,
    exports: BTreeMap<String, String>,
}

/// A complete, verified single-Library bundle with immutable exact content bytes.
#[derive(Debug, Clone)]
pub struct AuthoredLibrary {
    manifest: Vec<u8>,
    ir: Vec<u8>,
    metadata: NormalizedMetadata,
}
impl AuthoredLibrary {
    /// Derive a manifest from authoring fields and verify it against the exact IR.
    ///
    /// ```
    /// use morphir_package::authoring::AuthoredLibrary;
    /// assert!(AuthoredLibrary::create(b"{}", b"{}").is_err());
    /// ```
    pub fn create(input: &[u8], ir: &[u8]) -> Result<Self, Error> {
        bounded(input, ir)?;
        let input: Input =
            serde_json::from_value(parse(input)?).map_err(|_| Error::InvalidInput)?;
        PackagePath::parse(&input.package_path).map_err(|_| Error::InvalidInput)?;
        StableVersion::parse(&input.version).map_err(|_| Error::InvalidInput)?;
        if !input.dependencies.is_empty() {
            return Err(Error::InvalidInput);
        }
        let parsed = parse(ir)?;
        let package = parsed
            .pointer("/distribution/Library/packageName")
            .and_then(Value::as_str)
            .ok_or(Error::InvalidInput)?;
        let manifest = json!({"formatVersion":"0.1.0-draft.1","kind":"Library",
            "packagePath":input.package_path,"version":input.version,
            "ir":{"formatVersion":"4","packageName":package,
                "payload":{"path":"ir.json","mediaType":"application/json","profile":"classic"}},
            "dependencies":{},"exports":input.exports,
            "content":{"ir.json":Digest::of_bytes(ir).to_string()}});
        let bytes = serde_json::to_vec(&manifest).map_err(|_| Error::InvalidInput)?;
        Self::from_bundle(&bytes, ir)
    }
    /// Revalidate a persisted bundle before signing. Never trust the directory name.
    pub fn from_bundle(manifest: &[u8], ir: &[u8]) -> Result<Self, Error> {
        bounded(manifest, ir)?;
        let text = std::str::from_utf8(manifest).map_err(|_| Error::InvalidInput)?;
        let metadata = NormalizedMetadata::parse(text).map_err(|_| Error::InvalidInput)?;
        let value = metadata.value();
        if value["dependencies"]
            .as_object()
            .is_none_or(|v| !v.is_empty())
            || value["ir"]["payload"]["path"] != "ir.json"
            || value["ir"]["payload"]["profile"] != "classic"
            || value["content"].as_object().is_none_or(|v| v.len() != 1)
        {
            return Err(Error::InvalidInput);
        }
        let lock = json!({"formatVersion":"0.1.0-draft.1","kind":"LibraryLockCore",
            "root":"n0","nodes":{"n0":{
                "release":{"packagePath":value["packagePath"],"version":value["version"]},
                "irPackageName":value["ir"]["packageName"],
                "manifestDigest":metadata.manifest_digest().to_string(),
                "contentDigest":metadata.content_digest().to_string(),"bindings":{}}}});
        let schemas = PackageSchemas::compile(
            &serde_json::from_str(include_str!(
                "../local_registry/mvp/schemas/library-manifest.schema.json"
            ))
            .map_err(|_| Error::InvalidInput)?,
            &serde_json::from_str(include_str!(
                "../local_registry/mvp/schemas/lock-core.schema.json"
            ))
            .map_err(|_| Error::InvalidInput)?,
        )
        .map_err(|_| Error::InvalidInput)?;
        VerifiedLibrarySet::verify(
            &schemas,
            &lock.to_string(),
            &[LibraryInput::new(
                text.to_owned(),
                vec![("ir.json".to_owned(), ir.to_vec())],
            )],
        )
        .map_err(|_| Error::InvalidInput)?;
        Ok(Self {
            manifest: manifest.to_vec(),
            ir: ir.to_vec(),
            metadata,
        })
    }
    /// Exact manifest bytes supplied to verification.
    pub fn manifest_bytes(&self) -> &[u8] {
        &self.manifest
    }
    /// Exact compiled IR bytes; authoring never rewrites them.
    pub fn ir_bytes(&self) -> &[u8] {
        &self.ir
    }
    /// Verified normalized manifest and domain-separated content identity.
    pub fn metadata(&self) -> &NormalizedMetadata {
        &self.metadata
    }
}
fn bounded(manifest: &[u8], ir: &[u8]) -> Result<(), Error> {
    if manifest.len() > 1_048_576 || ir.len() > 64 * 1024 * 1024 {
        Err(Error::ResourceLimit)
    } else {
        Ok(())
    }
}
fn parse(bytes: &[u8]) -> Result<Value, Error> {
    strict_json::parse(std::str::from_utf8(bytes).map_err(|_| Error::InvalidInput)?)
        .map_err(|_| Error::InvalidInput)
}

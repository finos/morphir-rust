// Frozen lock and provenance DTOs shared by CLI beta.5, beta.6 and beta.7.
// morphir-rust revisions: c16d6f1, e0318dc, bfbb76d respectively.
use morphir_distribution::{
    ArtifactRuntime, ArtifactSource, BackendRecord, Capability, ExtensionId, FrontendRecord,
    IndexKind, Platform, SchemaVersion, Selection, Sha256Digest,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Reproducible selection and integrity record for one installed extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionLock {
    schema_version: SchemaVersion,
    selection: Selection,
    extension_id: ExtensionId,
    name: String,
    version: Version,
    index: IndexProvenance,
    source: ArtifactSource,
    runtime: ArtifactRuntime,
    platform: Option<Platform>,
    args: Vec<String>,
    digest: Sha256Digest,
    capabilities: Vec<Capability>,
    mep_versions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    frontend: Option<FrontendRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    backend: Option<BackendRecord>,
    executable: bool,
}

/// Exact index metadata used to resolve an artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexProvenance {
    kind: IndexKind,
    identity: PathBuf,
    revision: Sha256Digest,
}

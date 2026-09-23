//! Pure readers for extension bundle descriptors.

use super::{LegacyReleaseBundleDescriptor, invalid_bundle};
use crate::domain::portable_token;
use crate::extension_format::{
    ExtensionSchemaVersion, StatementProvenance, StatementRecord, validate_members,
};
use crate::{ArtifactFilename, ArtifactRuntime, ExtensionId, Result, Sha256Digest};
use morphir_extension_sdk::statement::CapabilityStatement;
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::path::Path;

const COMMON_PATHS: &[&str] = &[
    "schemaVersion",
    "extensionId",
    "shortId",
    "version",
    "gitCommit",
    "critical",
    "requires",
    "requires.host",
];
const LEGACY_PATHS: &[&str] = &[
    "package",
    "name",
    "mepVersions",
    "runtime",
    "targets",
    "languages",
    "languages.id",
    "languages.fileExtensions",
    "incremental",
    "workspaceDiscovery",
    "irVersions",
    "artifact",
    "sha256",
    "statement",
    "statementSource",
];
const V2_PATHS: &[&str] = &[
    "platformDifferences",
    "artifacts",
    "artifacts.platform",
    "artifacts.runtime",
    "artifacts.filename",
    "artifacts.sha256",
    "artifacts.statement",
    "artifacts.statementSource",
    "artifacts.critical",
    "artifacts.requires",
    "artifacts.requires.host",
];
const ARTIFACT_PATHS: &[&str] = &[
    "platform",
    "runtime",
    "filename",
    "sha256",
    "statement",
    "statementSource",
    "critical",
    "requires",
    "requires.host",
];

/// Whether a publisher permits capability differences between artifact platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlatformDifferences {
    /// All artifacts are expected to report the same capabilities.
    None,
    /// The publisher intentionally declares platform-specific capabilities.
    Declared,
}

/// A bundle descriptor normalized to per-artifact capability statements.
///
/// Reading a descriptor does not open, verify, execute, or publish its artifacts.
///
/// ```
/// use morphir_distribution::ReleaseBundleDescriptor;
/// let descriptor = br#"{
///   "schemaVersion": 1, "shortId": "sql", "extensionId": "morphir-sql",
///   "package": "morphir-sql-extension", "version": "0.1.0",
///   "runtime": "wasm", "mepVersions": ["0.1"], "targets": ["sql"],
///   "irVersions": ["3"], "artifact": "sql.wasm",
///   "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
/// }"#;
/// let release = ReleaseBundleDescriptor::parse_json(descriptor).unwrap();
/// assert_eq!(release.artifacts().len(), 1);
/// ```
#[derive(Debug)]
pub struct ReleaseBundleDescriptor {
    schema_version: ExtensionSchemaVersion,
    extension_id: ExtensionId,
    short_id: String,
    version: Version,
    artifacts: Vec<BundleArtifactDescriptor>,
    platform_differences: Option<PlatformDifferences>,
    legacy: Option<LegacyReleaseBundleDescriptor>,
    wire: Value,
}

/// One artifact named by a bundle descriptor, including its unprobed statement.
#[derive(Debug)]
pub struct BundleArtifactDescriptor {
    runtime: ArtifactRuntime,
    platform: Option<String>,
    filename: ArtifactFilename,
    sha256: Sha256Digest,
    statement_record: StatementRecord,
}

impl ReleaseBundleDescriptor {
    /// Parse a descriptor without accessing the filesystem or probing a guest.
    pub fn parse_json(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes)
            .map_err(|error| invalid_bundle("release.json", error.to_string()))
    }

    /// Return the validated descriptor format version.
    pub fn schema_version(&self) -> &ExtensionSchemaVersion {
        &self.schema_version
    }
    /// Return the extension identity.
    pub fn extension_id(&self) -> &ExtensionId {
        &self.extension_id
    }
    /// Return the portable short identifier.
    pub fn short_id(&self) -> &str {
        &self.short_id
    }
    /// Return the release version.
    pub fn version(&self) -> &Version {
        &self.version
    }
    /// Return artifact declarations in descriptor order.
    pub fn artifacts(&self) -> &[BundleArtifactDescriptor] {
        &self.artifacts
    }
    /// Return the publisher's declared policy for platform capability differences.
    pub fn platform_differences(&self) -> Option<PlatformDifferences> {
        self.platform_differences
    }

    pub(super) fn into_legacy(self, root: &Path) -> Result<LegacyReleaseBundleDescriptor> {
        self.legacy.ok_or_else(|| {
            invalid_bundle(
                root.join("release.json"),
                "local repository publication currently requires a version-1 WASM bundle",
            )
        })
    }

    fn from_value(mut value: Value) -> std::result::Result<Self, String> {
        let original = value.clone();
        if value
            .get("schemaVersion")
            .is_some_and(|version| version == 1)
        {
            value["schemaVersion"] = Value::String("1.0".into());
        }
        let schema_version: ExtensionSchemaVersion = value
            .get("schemaVersion")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        let is_v2 = matches!(&schema_version, ExtensionSchemaVersion::Semver(version) if version.major == 2);
        let paths: Vec<_> = COMMON_PATHS
            .iter()
            .chain(if is_v2 { V2_PATHS } else { LEGACY_PATHS })
            .copied()
            .collect();
        validate_members(&value, &paths, true)?;
        if !is_v2 {
            let legacy: LegacyReleaseBundleDescriptor =
                serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
            legacy
                .validate(Path::new(""))
                .map_err(|error| error.to_string())?;
            let mut statement_record = legacy.statement_record.clone();
            let release = legacy
                .release_record(Path::new(""))
                .map_err(|error| error.to_string())?;
            statement_record.supply_legacy(
                release.artifacts()[0]
                    .statement()
                    .expect("release reader supplies a legacy statement")
                    .clone(),
            );
            let artifact = BundleArtifactDescriptor {
                runtime: legacy.runtime,
                platform: None,
                filename: legacy.artifact.clone(),
                sha256: legacy.sha256.clone(),
                statement_record,
            };
            return Ok(Self {
                schema_version,
                extension_id: legacy.extension_id.clone(),
                short_id: legacy.short_id.clone(),
                version: legacy.version.clone(),
                artifacts: vec![artifact],
                platform_differences: None,
                legacy: Some(legacy),
                wire: original,
            });
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct V2 {
            extension_id: ExtensionId,
            short_id: String,
            version: Version,
            artifacts: Vec<Value>,
            #[serde(default)]
            platform_differences: Option<PlatformDifferences>,
        }
        let wire: V2 = serde_json::from_value(value).map_err(|error| error.to_string())?;
        if !portable_token(&wire.short_id) {
            return Err("release bundle shortId must be a portable token".into());
        }
        if wire.artifacts.is_empty() {
            return Err("release bundle must contain at least one artifact".into());
        }
        let artifacts = wire
            .artifacts
            .into_iter()
            .map(|artifact| serde_json::from_value(artifact).map_err(|error| error.to_string()))
            .collect::<std::result::Result<Vec<_>, String>>()?;
        Ok(Self {
            schema_version,
            extension_id: wire.extension_id,
            short_id: wire.short_id,
            version: wire.version,
            artifacts,
            platform_differences: wire.platform_differences,
            legacy: None,
            wire: original,
        })
    }
}

impl Serialize for ReleaseBundleDescriptor {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        self.wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ReleaseBundleDescriptor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        Self::from_value(Value::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl<'de> Deserialize<'de> for BundleArtifactDescriptor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        validate_members(&value, ARTIFACT_PATHS, true).map_err(serde::de::Error::custom)?;
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            runtime: ArtifactRuntime,
            #[serde(default)]
            platform: Option<String>,
            filename: ArtifactFilename,
            sha256: Sha256Digest,
            #[serde(flatten)]
            statement_record: StatementRecord,
        }
        let platform_present = value.get("platform").is_some();
        let wire: Wire = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        if wire.statement_record.statement().is_none() {
            return Err(serde::de::Error::custom(
                "version-2 bundle artifact requires statement",
            ));
        }
        if wire.runtime == ArtifactRuntime::Process
            && wire
                .platform
                .as_ref()
                .is_none_or(|platform| platform.trim().is_empty())
        {
            return Err(serde::de::Error::custom(
                "process bundle artifact requires platform",
            ));
        }
        if wire.runtime == ArtifactRuntime::Wasm && platform_present {
            return Err(serde::de::Error::custom(
                "WASM bundle artifact must omit platform",
            ));
        }
        Ok(Self {
            runtime: wire.runtime,
            platform: wire.platform,
            filename: wire.filename,
            sha256: wire.sha256,
            statement_record: wire.statement_record,
        })
    }
}

impl BundleArtifactDescriptor {
    /// Return the artifact runtime.
    pub fn runtime(&self) -> ArtifactRuntime {
        self.runtime
    }
    /// Return the process target triple, absent for portable WASM artifacts.
    pub fn platform(&self) -> Option<&str> {
        self.platform.as_deref()
    }
    /// Return the portable artifact filename.
    pub fn filename(&self) -> &ArtifactFilename {
        &self.filename
    }
    /// Return the artifact's declared SHA-256 digest.
    pub fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }
    /// Return the supplied statement or a declared conversion of legacy metadata.
    pub fn statement(&self) -> &CapabilityStatement {
        self.statement_record
            .statement()
            .expect("bundle reader requires or supplies a statement")
    }
    /// Return whether the statement was declared or recorded as probed.
    pub fn statement_provenance(&self) -> StatementProvenance {
        self.statement_record.provenance()
    }
}

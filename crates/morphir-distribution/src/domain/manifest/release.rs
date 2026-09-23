//! Extension index records and conversion of legacy capability declarations.

use super::*;

/// One exact extension release from a JSONL history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseRecord {
    #[serde(default)]
    schema_version: ExtensionSchemaVersion,
    id: ExtensionId,
    name: String,
    version: Version,
    channels: Vec<Channel>,
    mep_versions: Vec<String>,
    capabilities: Vec<Capability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frontend: Option<FrontendRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    backend: Option<BackendRecord>,
    artifacts: Vec<ArtifactRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    critical: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    requires: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseRecordWire {
    #[serde(default)]
    schema_version: ExtensionSchemaVersion,
    id: ExtensionId,
    name: String,
    version: Version,
    #[serde(default)]
    channels: Vec<Channel>,
    #[serde(default)]
    mep_versions: Vec<String>,
    #[serde(default)]
    capabilities: Vec<Capability>,
    #[serde(default)]
    frontend: FieldPresence<FrontendRecord>,
    #[serde(default)]
    backend: FieldPresence<BackendRecord>,
    artifacts: Vec<ArtifactRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    critical: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    requires: Option<serde_json::Value>,
}

impl ReleaseRecord {
    /// Return the index record schema version.
    pub fn schema_version(&self) -> ExtensionSchemaVersion {
        self.schema_version.clone()
    }

    /// Return the stable portable identity.
    pub fn extension_id(&self) -> &ExtensionId {
        &self.id
    }

    /// Return the non-empty human-readable name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the exact semantic version.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// Return moving channels that point at this release.
    pub fn channels(&self) -> &[Channel] {
        &self.channels
    }

    /// Return legacy MEP declarations. Statement-only records declare versions per artifact.
    pub fn mep_versions(&self) -> &[String] {
        &self.mep_versions
    }

    /// Return legacy capability kinds. Statement-only records declare kinds per artifact.
    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    /// Return frontend-specific metadata when declared by a schema `"1.0"` record.
    pub fn frontend(&self) -> Option<&FrontendRecord> {
        self.frontend.as_ref()
    }

    /// Return backend-specific metadata when declared by a schema `"1.0"` record.
    pub fn backend(&self) -> Option<&BackendRecord> {
        self.backend.as_ref()
    }

    /// Return the non-empty platform artifact set.
    pub fn artifacts(&self) -> &[ArtifactRecord] {
        &self.artifacts
    }
}

impl<'de> Deserialize<'de> for ReleaseRecord {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let mut paths = RELEASE_PATHS.to_vec();
        paths.extend_from_slice(CAPABILITY_PATHS);
        validate_members(&value, &paths, true).map_err(serde::de::Error::custom)?;
        let mut wire: ReleaseRecordWire =
            serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        if wire.name.trim().is_empty() {
            return Err(serde::de::Error::custom("extension name cannot be empty"));
        }
        let has_statements = !wire.artifacts.is_empty()
            && wire
                .artifacts
                .iter()
                .all(|artifact| artifact.statement().is_some());
        if (!has_statements && wire.mep_versions.is_empty())
            || wire
                .mep_versions
                .iter()
                .any(|version| version.trim().is_empty())
        {
            return Err(serde::de::Error::custom(
                "MEP versions must contain non-empty values",
            ));
        }
        if wire.capabilities.is_empty() && !has_statements {
            return Err(serde::de::Error::custom(
                "extension capabilities cannot be empty",
            ));
        }
        if wire.capabilities.iter().collect::<BTreeSet<_>>().len() != wire.capabilities.len() {
            return Err(serde::de::Error::custom(
                "extension capabilities cannot contain duplicates",
            ));
        }
        let declares_backend = wire.capabilities.contains(&Capability::Backend);
        match (declares_backend, wire.backend.has_value()) {
            (true, false) => {
                return Err(serde::de::Error::custom(
                    "backend metadata is required when backend capability is declared",
                ));
            }
            (false, _) if !wire.backend.is_missing() => {
                return Err(serde::de::Error::custom(
                    "backend metadata requires the backend capability",
                ));
            }
            _ => {}
        }
        let declares_frontend = wire.capabilities.contains(&Capability::Frontend);
        match (declares_frontend, wire.frontend.has_value()) {
            (true, false) => {
                return Err(serde::de::Error::custom(
                    "frontend metadata is required when frontend capability is declared",
                ));
            }
            (false, _) if !wire.frontend.is_missing() => {
                return Err(serde::de::Error::custom(
                    "frontend metadata requires the frontend capability",
                ));
            }
            _ => {}
        }
        if wire.artifacts.is_empty() {
            return Err(serde::de::Error::custom(
                "release artifacts cannot be empty",
            ));
        }
        let mut capabilities = serde_json::Map::new();
        if let Some(frontend) = wire.frontend.as_option() {
            capabilities.insert(
                "frontend".into(),
                serde_json::to_value(frontend).map_err(serde::de::Error::custom)?,
            );
        }
        if let Some(backend) = wire.backend.as_option() {
            capabilities.insert(
                "backend".into(),
                serde_json::to_value(backend).map_err(serde::de::Error::custom)?,
            );
        }
        let info = morphir_extension_sdk::ExtensionInfo {
            id: wire.id.to_string(),
            name: wire.name.clone(),
            version: wire.version.to_string(),
            types: serde_json::from_value(
                serde_json::to_value(&wire.capabilities).map_err(serde::de::Error::custom)?,
            )
            .map_err(serde::de::Error::custom)?,
            ..Default::default()
        };
        let statement =
            CapabilityStatement::from_session(wire.mep_versions.clone(), info, capabilities);
        for artifact in &mut wire.artifacts {
            artifact.statement.supply_legacy(statement.clone());
        }
        Ok(Self {
            schema_version: wire.schema_version,
            critical: wire.critical,
            requires: wire.requires,
            id: wire.id,
            name: wire.name,
            version: wire.version,
            channels: wire.channels,
            mep_versions: wire.mep_versions,
            capabilities: wire.capabilities,
            frontend: wire.frontend.into_option(),
            backend: wire.backend.into_option(),
            artifacts: wire.artifacts,
        })
    }
}

const RELEASE_PATHS: &[&str] = &[
    "schemaVersion",
    "id",
    "name",
    "version",
    "channels",
    "mepVersions",
    "capabilities",
    "artifacts",
    "critical",
    "requires",
    "requires.host",
    "artifacts.runtime",
    "artifacts.platform",
    "artifacts.platform.os",
    "artifacts.platform.arch",
    "artifacts.source",
    "artifacts.source.kind",
    "artifacts.source.path",
    "artifacts.sha256",
    "artifacts.filename",
    "artifacts.args",
    "artifacts.executable",
    "artifacts.statement",
    "artifacts.statementSource",
    "artifacts.critical",
];

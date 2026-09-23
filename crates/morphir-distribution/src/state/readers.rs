//! Must-ignore installed extension catalog readers.

use super::*;
use crate::extension_format::{CAPABILITY_PATHS, validate_members};
use serde::de::Error as _;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstalledExtensionWire {
    extension_id: ExtensionId,
    name: String,
    version: Version,
    runtime: ArtifactRuntime,
    #[serde(default, deserialize_with = "crate::extension_format::read_platform")]
    platform: Option<Platform>,
    args: Vec<String>,
    digest: Sha256Digest,
    store_path: RelativeArtifactPath,
    capabilities: Vec<Capability>,
    mep_versions: Vec<String>,
    index: IndexProvenance,
    #[serde(default)]
    frontend: Option<FrontendRecord>,
    #[serde(default)]
    backend: Option<BackendRecord>,
    executable: bool,
    #[serde(flatten)]
    statement: StatementRecord,
    #[serde(default)]
    critical: Vec<String>,
    #[serde(default)]
    requires: Option<Value>,
}

const INSTALLED_PATHS: &[&str] = &[
    "extensionId",
    "name",
    "version",
    "runtime",
    "platform",
    "platform.os",
    "platform.arch",
    "args",
    "digest",
    "storePath",
    "capabilities",
    "mepVersions",
    "index",
    "index.kind",
    "index.identity",
    "index.revision",
    "executable",
    "statement",
    "statementSource",
    "critical",
    "requires",
    "requires.host",
];

impl<'de> Deserialize<'de> for InstalledExtension {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let mut value = Value::deserialize(deserializer)?;
        let mut paths = INSTALLED_PATHS.to_vec();
        paths.extend_from_slice(CAPABILITY_PATHS);
        validate_members(&value, &paths, true).map_err(D::Error::custom)?;
        if let Some(statement) = value.get("statement").cloned() {
            let object = value
                .as_object_mut()
                .ok_or_else(|| D::Error::custom("expected installed record"))?;
            for (flat, projected) in [
                ("mepVersions", &statement["protocolVersions"]),
                ("capabilities", &statement["extension"]["types"]),
                ("frontend", &statement["capabilities"]["frontend"]),
                ("backend", &statement["capabilities"]["backend"]),
            ] {
                if !projected.is_null() {
                    object.entry(flat).or_insert_with(|| projected.clone());
                }
            }
        }
        let wire: InstalledExtensionWire =
            serde_json::from_value(value).map_err(D::Error::custom)?;
        let mut installed = Self {
            extension_id: wire.extension_id,
            name: wire.name,
            version: wire.version,
            runtime: wire.runtime,
            platform: wire.platform,
            args: wire.args,
            digest: wire.digest,
            store_path: wire.store_path,
            capabilities: wire.capabilities,
            mep_versions: wire.mep_versions,
            index: wire.index,
            frontend: wire.frontend,
            backend: wire.backend,
            executable: wire.executable,
            statement: wire.statement,
            critical: wire.critical,
            requires: wire.requires,
        };
        let mut capabilities = serde_json::Map::new();
        if let Some(frontend) = &installed.frontend {
            capabilities.insert(
                "frontend".into(),
                serde_json::to_value(frontend).map_err(D::Error::custom)?,
            );
        }
        if let Some(backend) = &installed.backend {
            capabilities.insert(
                "backend".into(),
                serde_json::to_value(backend).map_err(D::Error::custom)?,
            );
        }
        installed
            .statement
            .supply_legacy(CapabilityStatement::from_session(
                installed.mep_versions.clone(),
                installed.extension_info(),
                capabilities,
            ));
        Ok(installed)
    }
}

impl<'de> Deserialize<'de> for CatalogFile {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            #[serde(default)]
            schema_version: ExtensionSchemaVersion,
            extensions: Vec<InstalledExtension>,
            #[serde(default)]
            critical: Vec<String>,
        }
        let value = Value::deserialize(deserializer)?;
        let mut paths = vec![
            "schemaVersion".to_owned(),
            "extensions".into(),
            "critical".into(),
        ];
        paths.extend(
            INSTALLED_PATHS
                .iter()
                .chain(CAPABILITY_PATHS)
                .map(|path| format!("extensions.{path}")),
        );
        validate_members(
            &value,
            &paths.iter().map(String::as_str).collect::<Vec<_>>(),
            false,
        )
        .map_err(D::Error::custom)?;
        let wire: Wire = serde_json::from_value(value).map_err(D::Error::custom)?;
        Ok(Self {
            schema_version: wire.schema_version,
            extensions: wire.extensions,
            critical: wire.critical,
        })
    }
}

/// Project only the selected artifact into the existing version-1 lock fields.
/// Statements from other platforms never contribute capabilities here.
pub(super) struct SelectedMetadata {
    pub capabilities: Vec<Capability>,
    pub mep_versions: Vec<String>,
    pub frontend: Option<FrontendRecord>,
    pub backend: Option<BackendRecord>,
}

impl SelectedMetadata {
    pub fn from_artifact(artifact: &VerifiedArtifact) -> Result<Self> {
        let statement = artifact
            .selected
            .artifact
            .statement()
            .expect("release reader supplies statement");
        Ok(Self {
            capabilities: serde_json::from_value(
                serde_json::to_value(&statement.extension.types)
                    .map_err(DistributionError::StateEncoding)?,
            )
            .map_err(DistributionError::StateEncoding)?,
            mep_versions: statement.protocol_versions.clone(),
            frontend: statement
                .capabilities
                .get("frontend")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(DistributionError::StateEncoding)?,
            backend: statement
                .capabilities
                .get("backend")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(DistributionError::StateEncoding)?,
        })
    }
}

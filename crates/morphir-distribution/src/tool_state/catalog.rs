//! Installed tool catalog types and atomic state transactions.

use super::package::{ToolPackageFile, VerifiedToolPackage};
use super::verification::verify_package;
use crate::state_io::{
    FilesystemStateWriter, StateGuard, StateWriter, commit_state_pair, decode_state, encode_json,
    read_json, read_state_bytes,
};
use crate::{
    DistributionError, Platform, RelativeArtifactPath, Result, Selection, Sha256Digest, ToolId,
    ToolReleaseStatus,
};
use morphir_common::home::MorphirHome;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const TOOL_LOCK_SCHEMA_VERSION: u32 = 1;
pub(super) const TOOL_CATALOG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateSchemaEnvelope {
    schema_version: u32,
}

/// Reproducible selection and integrity record for one active tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolLock {
    schema_version: u32,
    selection: Selection,
    tool_id: ToolId,
    tool_name: String,
    version: Version,
    status: ToolReleaseStatus,
    platform: Platform,
    digest: Sha256Digest,
    length: u64,
    targets_version: u64,
    target_path: RelativeArtifactPath,
    store_path: RelativeArtifactPath,
    args: Vec<String>,
    files: Vec<ToolPackageFile>,
}

impl ToolLock {
    fn from_package(package: &VerifiedToolPackage) -> Self {
        Self {
            schema_version: TOOL_LOCK_SCHEMA_VERSION,
            selection: package.selection.clone(),
            tool_id: package.tool_id.clone(),
            tool_name: package.tool_name.clone(),
            version: package.version.clone(),
            status: package.status,
            platform: package.platform.clone(),
            digest: package.digest.clone(),
            length: package.length,
            targets_version: package.targets_version,
            target_path: package.target_path.clone(),
            store_path: package.store_path.clone(),
            args: package.args.clone(),
            files: package.files.clone(),
        }
    }

    pub(super) fn from_installed(installed: &InstalledTool) -> Self {
        Self {
            schema_version: TOOL_LOCK_SCHEMA_VERSION,
            selection: installed.selection.clone(),
            tool_id: installed.tool_id.clone(),
            tool_name: installed.tool_name.clone(),
            version: installed.version.clone(),
            status: installed.status,
            platform: installed.platform.clone(),
            digest: installed.digest.clone(),
            length: installed.length,
            targets_version: installed.targets_version,
            target_path: installed.target_path.clone(),
            store_path: installed.store_path.clone(),
            args: installed.args.clone(),
            files: installed.files.clone(),
        }
    }

    /// Return the requested channel or exact version.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Return the exact selected tool identity.
    pub fn tool_id(&self) -> &ToolId {
        &self.tool_id
    }

    /// Return the exact selected semantic version.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// Return the authenticated artifact digest.
    pub fn digest(&self) -> &Sha256Digest {
        &self.digest
    }
}

/// One immutable installed tool release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstalledTool {
    pub(super) selection: Selection,
    pub(super) tool_id: ToolId,
    pub(super) tool_name: String,
    pub(super) version: Version,
    pub(super) status: ToolReleaseStatus,
    pub(super) platform: Platform,
    pub(super) digest: Sha256Digest,
    pub(super) length: u64,
    pub(super) targets_version: u64,
    pub(super) target_path: RelativeArtifactPath,
    pub(super) store_path: RelativeArtifactPath,
    pub(super) args: Vec<String>,
    pub(super) files: Vec<ToolPackageFile>,
}

impl InstalledTool {
    fn from_package(package: &VerifiedToolPackage) -> Self {
        Self {
            selection: package.selection.clone(),
            tool_id: package.tool_id.clone(),
            tool_name: package.tool_name.clone(),
            version: package.version.clone(),
            status: package.status,
            platform: package.platform.clone(),
            digest: package.digest.clone(),
            length: package.length,
            targets_version: package.targets_version,
            target_path: package.target_path.clone(),
            store_path: package.store_path.clone(),
            args: package.args.clone(),
            files: package.files.clone(),
        }
    }

    /// Return the stable tool identity.
    pub fn tool_id(&self) -> &ToolId {
        &self.tool_id
    }

    /// Return the channel or exact version request that selected this release.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Return the human-readable tool name.
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    /// Return the exact installed semantic version.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// Return the installed platform.
    pub fn platform(&self) -> &Platform {
        &self.platform
    }

    /// Return the artifact digest authenticated at installation time.
    pub fn digest(&self) -> &Sha256Digest {
        &self.digest
    }

    /// Return the installed program path relative to Morphir Home.
    pub fn store_path(&self) -> &Path {
        self.store_path.as_path()
    }

    /// Return fixed arguments prepended during launch.
    pub fn args(&self) -> &[String] {
        &self.args
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ToolCatalogEntry {
    pub(super) active: InstalledTool,
    pub(super) rollback: Vec<InstalledTool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ToolCatalogFile {
    pub(super) schema_version: u32,
    pub(super) tools: Vec<ToolCatalogEntry>,
}

/// One atomically read active tool, its exact selection, and retained rollback releases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledToolSnapshot {
    active: InstalledTool,
    rollback: Vec<InstalledTool>,
    selection: Selection,
}

impl InstalledToolSnapshot {
    /// Return the active exact release.
    pub fn active(&self) -> &InstalledTool {
        &self.active
    }

    /// Return inactive releases retained for explicit rollback and pruning policy.
    pub fn rollback(&self) -> &[InstalledTool] {
        &self.rollback
    }

    /// Return the channel or exact-version request stored in the exact lock.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }
}

/// Transactional activation of one already verified tool package.
#[derive(Debug)]
pub struct ToolInstaller<'home> {
    home: &'home MorphirHome,
}

impl<'home> ToolInstaller<'home> {
    /// Construct an installer for one Morphir Home.
    pub fn new(home: &'home MorphirHome) -> Self {
        Self { home }
    }

    /// Verify the candidate again, then atomically replace its lock and catalog entry.
    #[tracing::instrument(
        name = "morphir.tool.install",
        skip(self),
        fields(
            tool_id = %package.tool_id,
            version = %package.version,
            digest = %package.digest
        ),
        err
    )]
    pub fn install(&self, package: VerifiedToolPackage) -> Result<InstalledTool> {
        self.install_with_writer(package, &FilesystemStateWriter)
    }

    pub(super) fn install_with_writer(
        &self,
        package: VerifiedToolPackage,
        writer: &impl StateWriter,
    ) -> Result<InstalledTool> {
        verify_package(self.home, &package)?;
        if package.status == ToolReleaseStatus::Revoked {
            return Err(DistributionError::RevokedToolRelease {
                tool: package.tool_id,
                version: package.version,
            });
        }

        let _transaction = tool_state_guard(self.home)?;
        let mut tools = load_catalog_unlocked(self.home)?;
        let lock = ToolLock::from_package(&package);
        let active = InstalledTool::from_package(&package);
        let previous = tools.remove(&active.tool_id);
        let rollback = next_rollback(previous, &active);
        tools.insert(
            active.tool_id.clone(),
            ToolCatalogEntry {
                active: active.clone(),
                rollback,
            },
        );
        let stored = ToolCatalogFile {
            schema_version: TOOL_CATALOG_SCHEMA_VERSION,
            tools: tools.into_values().collect(),
        };
        commit_state_pair(
            &tool_lock_path(self.home, &active.tool_id),
            &encode_json(&lock)?,
            &self.home.tools_catalog_file(),
            &encode_json(&stored)?,
            writer,
        )?;
        tracing::info!(
            tool_id = %active.tool_id,
            version = %active.version,
            rollback_count = stored
                .tools
                .iter()
                .find(|entry| entry.active.tool_id == active.tool_id)
                .map_or(0, |entry| entry.rollback.len()),
            "tool catalog activation committed"
        );
        Ok(active)
    }
}

fn next_rollback(previous: Option<ToolCatalogEntry>, active: &InstalledTool) -> Vec<InstalledTool> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    let candidates = if previous.active == *active {
        previous.rollback
    } else {
        std::iter::once(previous.active)
            .chain(previous.rollback)
            .collect()
    };
    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|candidate| seen.insert((candidate.version.clone(), candidate.digest.clone())))
        .collect()
}

/// Read and validate the exact active lock for one tool.
pub fn read_tool_lock(home: &MorphirHome, id: &ToolId) -> Result<ToolLock> {
    let _transaction = tool_state_guard(home)?;
    read_tool_lock_unlocked(home, id)
}

pub(super) fn read_tool_lock_unlocked(home: &MorphirHome, id: &ToolId) -> Result<ToolLock> {
    let path = tool_lock_path(home, id);
    let bytes = read_state_bytes(&path)?;
    let envelope: StateSchemaEnvelope = decode_state(&path, &bytes)?;
    if envelope.schema_version != TOOL_LOCK_SCHEMA_VERSION {
        return Err(DistributionError::UnsupportedStateSchema {
            kind: "tool lock",
            version: envelope.schema_version,
        });
    }
    let lock: ToolLock = decode_state(&path, &bytes)?;
    if &lock.tool_id != id {
        return Err(DistributionError::ToolStateMismatch { id: id.clone() });
    }
    Ok(lock)
}

/// Atomically list active tools with their exact selections and rollback releases.
pub fn list_installed_tools(home: &MorphirHome) -> Result<Vec<InstalledToolSnapshot>> {
    let _transaction = tool_state_guard(home)?;
    load_catalog_unlocked(home)?
        .into_values()
        .map(|entry| {
            let lock = read_tool_lock_unlocked(home, entry.active.tool_id())?;
            validate_active_pair(&entry.active, &lock)?;
            Ok(InstalledToolSnapshot {
                active: entry.active,
                rollback: entry.rollback,
                selection: lock.selection,
            })
        })
        .collect()
}

pub(super) fn load_catalog_unlocked(
    home: &MorphirHome,
) -> Result<BTreeMap<ToolId, ToolCatalogEntry>> {
    let path = home.tools_catalog_file();
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let stored: ToolCatalogFile = read_json(&path)?;
    if stored.schema_version != TOOL_CATALOG_SCHEMA_VERSION {
        return Err(DistributionError::UnsupportedStateSchema {
            kind: "installed tool catalog",
            version: stored.schema_version,
        });
    }
    let mut tools = BTreeMap::new();
    for entry in stored.tools {
        let id = entry.active.tool_id.clone();
        if tools.insert(id.clone(), entry).is_some() {
            return Err(DistributionError::ToolStateMismatch { id });
        }
    }
    Ok(tools)
}

pub(super) fn validate_active_pair(active: &InstalledTool, lock: &ToolLock) -> Result<()> {
    let matches = lock.schema_version == TOOL_LOCK_SCHEMA_VERSION
        && lock.selection == active.selection
        && lock.tool_id == active.tool_id
        && lock.tool_name == active.tool_name
        && lock.version == active.version
        && lock.status == active.status
        && lock.platform == active.platform
        && lock.digest == active.digest
        && lock.length == active.length
        && lock.targets_version == active.targets_version
        && lock.target_path == active.target_path
        && lock.store_path == active.store_path
        && lock.args == active.args
        && lock.files == active.files;
    if matches {
        Ok(())
    } else {
        Err(DistributionError::ToolStateMismatch {
            id: active.tool_id.clone(),
        })
    }
}

pub(super) fn tool_state_guard(home: &MorphirHome) -> Result<StateGuard> {
    StateGuard::acquire(&home.tools_state_lock_file())
}

pub(super) fn tool_lock_path(home: &MorphirHome, id: &ToolId) -> PathBuf {
    home.tools_locks_dir().join(format!("{id}.json"))
}

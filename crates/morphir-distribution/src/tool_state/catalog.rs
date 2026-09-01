//! Installed tool catalog types and atomic state transactions.

use super::package::{ToolPackageFile, VerifiedToolPackage};
use super::verification::{sync_package, verify_package};
use crate::state_io::{
    FilesystemStateWriter, StateGuard, StateWriter, commit_state_pair, decode_state, encode_json,
    read_state_bytes, recover_state_pairs, remove_state_pair,
};
use crate::{
    DistributionError, Platform, RelativeArtifactPath, Result, Selection, Sha256Digest, ToolId,
    ToolProvenance, ToolReleaseStatus,
};
use morphir_common::home::MorphirHome;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const TOOL_LOCK_SCHEMA_VERSION: u32 = 2;
pub(super) const TOOL_CATALOG_SCHEMA_VERSION: u32 = 2;
const MAX_ROLLBACK_RELEASES: usize = 1;

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
    provenance: ToolProvenance,
    tool_id: ToolId,
    tool_name: String,
    version: Version,
    status: ToolReleaseStatus,
    platform: Platform,
    digest: Sha256Digest,
    length: u64,
    target_path: RelativeArtifactPath,
    store_path: RelativeArtifactPath,
    package_root: Option<RelativeArtifactPath>,
    args: Vec<String>,
    files: Vec<ToolPackageFile>,
    directories: Vec<RelativeArtifactPath>,
    rollback: Vec<InstalledTool>,
}

impl ToolLock {
    fn from_package(package: &VerifiedToolPackage, rollback: &[InstalledTool]) -> Self {
        Self {
            schema_version: TOOL_LOCK_SCHEMA_VERSION,
            selection: package.selection.clone(),
            provenance: package.provenance.clone(),
            tool_id: package.tool_id.clone(),
            tool_name: package.tool_name.clone(),
            version: package.version.clone(),
            status: package.status,
            platform: package.platform.clone(),
            digest: package.digest.clone(),
            length: package.length,
            target_path: package.target_path.clone(),
            store_path: package.store_path.clone(),
            package_root: package.package_root.clone(),
            args: package.args.clone(),
            files: package.files.clone(),
            directories: package.directories.clone(),
            rollback: rollback.to_vec(),
        }
    }

    pub(super) fn from_installed(installed: &InstalledTool, rollback: &[InstalledTool]) -> Self {
        Self {
            schema_version: TOOL_LOCK_SCHEMA_VERSION,
            selection: installed.selection.clone(),
            provenance: installed.provenance.clone(),
            tool_id: installed.tool_id.clone(),
            tool_name: installed.tool_name.clone(),
            version: installed.version.clone(),
            status: installed.status,
            platform: installed.platform.clone(),
            digest: installed.digest.clone(),
            length: installed.length,
            target_path: installed.target_path.clone(),
            store_path: installed.store_path.clone(),
            package_root: installed.package_root.clone(),
            args: installed.args.clone(),
            files: installed.files.clone(),
            directories: installed.directories.clone(),
            rollback: rollback.to_vec(),
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

    /// Return the trust boundary that produced this exact lock.
    pub fn provenance(&self) -> &ToolProvenance {
        &self.provenance
    }
}

/// One immutable installed tool release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstalledTool {
    pub(super) selection: Selection,
    pub(super) provenance: ToolProvenance,
    pub(super) tool_id: ToolId,
    pub(super) tool_name: String,
    pub(super) version: Version,
    pub(super) status: ToolReleaseStatus,
    pub(super) platform: Platform,
    pub(super) digest: Sha256Digest,
    pub(super) length: u64,
    pub(super) target_path: RelativeArtifactPath,
    pub(super) store_path: RelativeArtifactPath,
    pub(super) package_root: Option<RelativeArtifactPath>,
    pub(super) args: Vec<String>,
    pub(super) files: Vec<ToolPackageFile>,
    pub(super) directories: Vec<RelativeArtifactPath>,
}

impl InstalledTool {
    fn from_package(package: &VerifiedToolPackage) -> Self {
        Self {
            selection: package.selection.clone(),
            provenance: package.provenance.clone(),
            tool_id: package.tool_id.clone(),
            tool_name: package.tool_name.clone(),
            version: package.version.clone(),
            status: package.status,
            platform: package.platform.clone(),
            digest: package.digest.clone(),
            length: package.length,
            target_path: package.target_path.clone(),
            store_path: package.store_path.clone(),
            package_root: package.package_root.clone(),
            args: package.args.clone(),
            files: package.files.clone(),
            directories: package.directories.clone(),
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

    /// Return the trust boundary that produced this installation.
    pub fn provenance(&self) -> &ToolProvenance {
        &self.provenance
    }

    /// Return the TUF snapshot version when this installation came from a repository.
    pub fn snapshot_version(&self) -> Option<u64> {
        self.provenance.snapshot_version()
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VersionOneInstalledTool {
    selection: Selection,
    tool_id: ToolId,
    tool_name: String,
    version: Version,
    status: ToolReleaseStatus,
    platform: Platform,
    digest: Sha256Digest,
    length: u64,
    snapshot_version: u64,
    target_path: RelativeArtifactPath,
    store_path: RelativeArtifactPath,
    package_root: Option<RelativeArtifactPath>,
    args: Vec<String>,
    files: Vec<ToolPackageFile>,
    directories: Vec<RelativeArtifactPath>,
}

impl From<VersionOneInstalledTool> for InstalledTool {
    fn from(legacy: VersionOneInstalledTool) -> Self {
        let provenance = ToolProvenance::AuthenticatedRepository {
            selection: legacy.selection.clone(),
            snapshot_version: legacy.snapshot_version,
        };
        Self {
            selection: legacy.selection,
            provenance,
            tool_id: legacy.tool_id,
            tool_name: legacy.tool_name,
            version: legacy.version,
            status: legacy.status,
            platform: legacy.platform,
            digest: legacy.digest,
            length: legacy.length,
            target_path: legacy.target_path,
            store_path: legacy.store_path,
            package_root: legacy.package_root,
            args: legacy.args,
            files: legacy.files,
            directories: legacy.directories,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VersionOneToolCatalogEntry {
    active: VersionOneInstalledTool,
    rollback: Vec<VersionOneInstalledTool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VersionOneToolCatalogFile {
    schema_version: u32,
    tools: Vec<VersionOneToolCatalogEntry>,
}

impl From<VersionOneToolCatalogFile> for ToolCatalogFile {
    fn from(legacy: VersionOneToolCatalogFile) -> Self {
        Self {
            schema_version: TOOL_CATALOG_SCHEMA_VERSION,
            tools: legacy
                .tools
                .into_iter()
                .map(|entry| ToolCatalogEntry {
                    active: entry.active.into(),
                    rollback: entry.rollback.into_iter().map(Into::into).collect(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VersionOneToolLock {
    schema_version: u32,
    selection: Selection,
    tool_id: ToolId,
    tool_name: String,
    version: Version,
    status: ToolReleaseStatus,
    platform: Platform,
    digest: Sha256Digest,
    length: u64,
    snapshot_version: u64,
    target_path: RelativeArtifactPath,
    store_path: RelativeArtifactPath,
    package_root: Option<RelativeArtifactPath>,
    args: Vec<String>,
    files: Vec<ToolPackageFile>,
    directories: Vec<RelativeArtifactPath>,
    rollback: Vec<VersionOneInstalledTool>,
}

impl From<VersionOneToolLock> for ToolLock {
    fn from(legacy: VersionOneToolLock) -> Self {
        let provenance = ToolProvenance::AuthenticatedRepository {
            selection: legacy.selection.clone(),
            snapshot_version: legacy.snapshot_version,
        };
        Self {
            schema_version: TOOL_LOCK_SCHEMA_VERSION,
            selection: legacy.selection,
            provenance,
            tool_id: legacy.tool_id,
            tool_name: legacy.tool_name,
            version: legacy.version,
            status: legacy.status,
            platform: legacy.platform,
            digest: legacy.digest,
            length: legacy.length,
            target_path: legacy.target_path,
            store_path: legacy.store_path,
            package_root: legacy.package_root,
            args: legacy.args,
            files: legacy.files,
            directories: legacy.directories,
            rollback: legacy.rollback.into_iter().map(Into::into).collect(),
        }
    }
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
        mut package: VerifiedToolPackage,
        writer: &impl StateWriter,
    ) -> Result<InstalledTool> {
        let _transaction = match package.take_state_guard(self.home)? {
            Some(state_guard) => state_guard,
            None => tool_state_guard(self.home)?,
        };
        verify_package(self.home, &package)?;
        sync_package(self.home, &package)?;
        if package.status == ToolReleaseStatus::Revoked {
            return Err(DistributionError::RevokedToolRelease {
                tool: package.tool_id,
                version: package.version,
            });
        }

        let mut tools = load_catalog_unlocked(self.home)?;
        let active = InstalledTool::from_package(&package);
        let previous = tools.remove(&active.tool_id);
        let rollback = next_rollback(previous, &active);
        let lock = ToolLock::from_package(&package, &rollback);
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
        .filter(|candidate| {
            candidate.version != active.version || candidate.digest != active.digest
        })
        .filter(|candidate| seen.insert((candidate.version.clone(), candidate.digest.clone())))
        .take(MAX_ROLLBACK_RELEASES)
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
    let (lock, _) = decode_tool_lock(&path, &bytes)?;
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
            validate_active_pair(&entry.active, &entry.rollback, &lock)?;
            Ok(InstalledToolSnapshot {
                active: entry.active,
                rollback: entry.rollback,
                selection: lock.selection,
            })
        })
        .collect()
}

/// Atomically remove one tool's active selection, retained rollback, and exact lock.
///
/// Content-addressed package bytes remain cache-owned and are reclaimed by normal cache policy.
#[tracing::instrument(name = "morphir.tool.uninstall", skip(home), fields(tool_id = %id), err)]
pub fn uninstall_tool(home: &MorphirHome, id: &ToolId) -> Result<InstalledTool> {
    let _transaction = tool_state_guard(home)?;
    let mut tools = load_catalog_unlocked(home)?;
    let entry = tools
        .remove(id)
        .ok_or_else(|| DistributionError::ToolNotInstalled { id: id.clone() })?;
    let lock = read_tool_lock_unlocked(home, id)?;
    validate_active_pair(&entry.active, &entry.rollback, &lock)?;
    let stored = ToolCatalogFile {
        schema_version: TOOL_CATALOG_SCHEMA_VERSION,
        tools: tools.into_values().collect(),
    };
    remove_state_pair(
        &tool_lock_path(home, id),
        &home.tools_catalog_file(),
        &encode_json(&stored)?,
        &FilesystemStateWriter,
    )?;
    tracing::info!(
        tool_id = %entry.active.tool_id,
        version = %entry.active.version,
        "tool uninstalled"
    );
    Ok(entry.active)
}

pub(super) fn load_catalog_unlocked(
    home: &MorphirHome,
) -> Result<BTreeMap<ToolId, ToolCatalogEntry>> {
    let path = home.tools_catalog_file();
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let bytes = read_state_bytes(&path)?;
    let (stored, _) = decode_tool_catalog(&path, &bytes)?;
    let mut tools = BTreeMap::new();
    for entry in stored.tools {
        let id = entry.active.tool_id.clone();
        if tools.insert(id.clone(), entry).is_some() {
            return Err(DistributionError::ToolStateMismatch { id });
        }
    }
    Ok(tools)
}

pub(super) fn validate_active_pair(
    active: &InstalledTool,
    rollback: &[InstalledTool],
    lock: &ToolLock,
) -> Result<()> {
    let matches = lock.schema_version == TOOL_LOCK_SCHEMA_VERSION
        && lock.selection == active.selection
        && lock.provenance == active.provenance
        && lock.tool_id == active.tool_id
        && lock.tool_name == active.tool_name
        && lock.version == active.version
        && lock.status == active.status
        && lock.platform == active.platform
        && lock.digest == active.digest
        && lock.length == active.length
        && lock.target_path == active.target_path
        && lock.store_path == active.store_path
        && lock.package_root == active.package_root
        && lock.args == active.args
        && lock.files == active.files
        && lock.directories == active.directories
        && lock.rollback == rollback;
    if matches {
        Ok(())
    } else {
        Err(DistributionError::ToolStateMismatch {
            id: active.tool_id.clone(),
        })
    }
}

pub(super) fn tool_state_guard(home: &MorphirHome) -> Result<StateGuard> {
    let guard = StateGuard::acquire(&home.tools_state_lock_file())?;
    recover_state_pairs(&home.tools_locks_dir(), &home.tools_catalog_file())?;
    super::repair_journal::recover_tool_repairs(home)?;
    migrate_version_one_tool_state(home)?;
    Ok(guard)
}

fn decode_tool_catalog(path: &Path, bytes: &[u8]) -> Result<(ToolCatalogFile, bool)> {
    let envelope: StateSchemaEnvelope = decode_state(path, bytes)?;
    match envelope.schema_version {
        TOOL_CATALOG_SCHEMA_VERSION => decode_state(path, bytes).map(|state| (state, false)),
        1 => {
            let legacy: VersionOneToolCatalogFile = decode_state(path, bytes)?;
            debug_assert_eq!(legacy.schema_version, 1);
            Ok((legacy.into(), true))
        }
        version => Err(DistributionError::UnsupportedStateSchema {
            kind: "installed tool catalog",
            version,
        }),
    }
}

fn decode_tool_lock(path: &Path, bytes: &[u8]) -> Result<(ToolLock, bool)> {
    let envelope: StateSchemaEnvelope = decode_state(path, bytes)?;
    match envelope.schema_version {
        TOOL_LOCK_SCHEMA_VERSION => decode_state(path, bytes).map(|state| (state, false)),
        1 => {
            let legacy: VersionOneToolLock = decode_state(path, bytes)?;
            debug_assert_eq!(legacy.schema_version, 1);
            Ok((legacy.into(), true))
        }
        version => Err(DistributionError::UnsupportedStateSchema {
            kind: "tool lock",
            version,
        }),
    }
}

fn migrate_version_one_tool_state(home: &MorphirHome) -> Result<()> {
    let catalog_path = home.tools_catalog_file();
    if !catalog_path.exists() {
        return Ok(());
    }
    let catalog_bytes = read_state_bytes(&catalog_path)?;
    let (catalog, catalog_was_legacy) = decode_tool_catalog(&catalog_path, &catalog_bytes)?;
    let next_catalog = encode_json(&catalog)?;
    if catalog.tools.is_empty() && catalog_was_legacy {
        FilesystemStateWriter.write(&catalog_path, &next_catalog)?;
    }
    for entry in &catalog.tools {
        let lock_path = tool_lock_path(home, entry.active.tool_id());
        let lock_bytes = read_state_bytes(&lock_path)?;
        let (lock, lock_was_legacy) = decode_tool_lock(&lock_path, &lock_bytes)?;
        validate_active_pair(&entry.active, &entry.rollback, &lock)?;
        if catalog_was_legacy || lock_was_legacy {
            commit_state_pair(
                &lock_path,
                &encode_json(&lock)?,
                &catalog_path,
                &next_catalog,
                &FilesystemStateWriter,
            )?;
        }
    }
    if catalog_was_legacy {
        tracing::info!(
            tool_count = catalog.tools.len(),
            from_schema = 1,
            to_schema = TOOL_CATALOG_SCHEMA_VERSION,
            "installed tool state migrated"
        );
    }
    Ok(())
}

pub(super) fn tool_lock_path(home: &MorphirHome, id: &ToolId) -> PathBuf {
    home.tools_locks_dir().join(format!("{id}.json"))
}

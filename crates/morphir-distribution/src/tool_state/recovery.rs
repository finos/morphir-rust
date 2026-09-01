//! Exact-release repair and explicit rollback operations.

use super::catalog::{
    InstalledTool, TOOL_CATALOG_SCHEMA_VERSION, ToolCatalogFile, ToolLock, load_catalog_unlocked,
    read_tool_lock_unlocked, tool_lock_path, tool_state_guard, validate_active_pair,
};
use super::local_package::LocalDeveloperToolPackage;
use super::package::{ToolPackageCandidate, ToolPackageStore, VerifiedToolPackage};
use super::repair_journal::{
    begin_repair, commit_repair, quarantined_digest_path, rollback_repair,
};
use super::verification::{sync_installed_file, sync_package, verify_installed, verify_package};
use crate::state_io::{
    FilesystemStateWriter, StateWriter, commit_state_pair, create_dir_all_durable, encode_json,
};
use crate::{
    DistributionError, DownloadedToolArtifact, RelativeArtifactPath, ResolvedTrustedToolArtifact,
    Result, ToolId,
};
use morphir_common::home::MorphirHome;
use std::fs;
use std::path::{Path, PathBuf};

/// Rebuilds the bytes for an installed exact release without changing its durable selection.
#[derive(Debug)]
pub struct ToolRepairer<'home> {
    home: &'home MorphirHome,
}

impl<'home> ToolRepairer<'home> {
    /// Construct a repairer for one Morphir Home.
    pub fn new(home: &'home MorphirHome) -> Self {
        Self { home }
    }

    /// Replace missing or corrupt active bytes from an authenticated exact-release download.
    ///
    /// The installed catalog and lock remain unchanged. The old bytes are quarantined until
    /// the replacement has been materialized and verified, then restored if any step fails.
    ///
    /// ```no_run
    /// use morphir_common::home::MorphirHome;
    /// use morphir_distribution::{
    ///     Platform, Selection, ToolId, ToolRepairer, TrustedToolRepository,
    ///     list_installed_tools,
    /// };
    /// use semver::Version;
    ///
    /// # async fn repair_desktop(
    /// #     home: &MorphirHome,
    /// #     repository: &TrustedToolRepository,
    /// # ) -> Result<(), Box<dyn std::error::Error>> {
    /// let id = ToolId::parse("desktop")?;
    /// let installed = list_installed_tools(home)?
    ///     .into_iter()
    ///     .find(|entry| entry.active().tool_id() == &id)
    ///     .ok_or("desktop is not installed")?;
    /// let version = installed.active().version().clone();
    /// let resolved = repository
    ///     .resolve(
    ///         &id,
    ///         &Selection::Exact(version.clone()),
    ///         &Platform::current(),
    ///         &Version::parse("0.4.0")?,
    ///     )
    ///     .await?;
    /// let staging = home.temp_dir().join("desktop-repair");
    /// let downloaded = repository.download(&resolved, &staging).await?;
    /// let repaired = ToolRepairer::new(home).repair(&id, resolved, downloaded)?;
    /// assert_eq!(repaired.version(), &version);
    /// # Ok(())
    /// # }
    /// ```
    #[tracing::instrument(
        name = "morphir.tool.repair",
        skip(self, resolved, downloaded),
        fields(tool_id = %id),
        err
    )]
    pub fn repair(
        &self,
        id: &ToolId,
        resolved: ResolvedTrustedToolArtifact,
        downloaded: DownloadedToolArtifact,
    ) -> Result<InstalledTool> {
        let _transaction = tool_state_guard(self.home)?;
        let tools = load_catalog_unlocked(self.home)?;
        let entry = tools
            .get(id)
            .ok_or_else(|| DistributionError::ToolNotInstalled { id: id.clone() })?;
        let active = entry.active.clone();
        let lock = read_tool_lock_unlocked(self.home, id)?;
        validate_active_pair(&active, &entry.rollback, &lock)?;
        let shared_files = shared_digest_files(self.home, &tools, &active);
        let shared_directories = shared_digest_directories(self.home, &tools, &active);
        validate_repair_resolution(&active, &resolved)?;
        let transaction = begin_repair(self.home, &active)?;
        let digest_path = self.home.tools_store_dir().join(active.digest.to_string());
        let previous_path = quarantined_digest_path(self.home, id, &active.digest);

        let repair = ToolPackageStore::new(self.home)
            .prepare_unlocked(resolved, downloaded)
            .and_then(|package| {
                validate_repair_package(&active, &package)?;
                verify_package(self.home, &package)?;
                sync_package(self.home, &package)?;
                restore_shared_directories(&digest_path, &previous_path, &shared_directories)?;
                restore_shared_files(self.home, &digest_path, &previous_path, &shared_files)?;
                Ok(())
            });

        if let Err(original) = repair {
            if let Err(rollback) = rollback_repair(self.home, &transaction) {
                return Err(DistributionError::StateRollback {
                    original: Box::new(original),
                    rollback: Box::new(rollback),
                });
            }
            return Err(original);
        }
        commit_repair(self.home, transaction)?;

        tracing::info!(
            tool_id = %active.tool_id,
            version = %active.version,
            digest = %active.digest,
            "installed tool bytes repaired"
        );
        Ok(active)
    }

    /// Repair an installed local-developer release from the explicitly supplied source package.
    #[tracing::instrument(
        name = "morphir.tool.repair_local",
        skip(self, local),
        fields(tool_id = %id),
        err
    )]
    pub fn repair_local(
        &self,
        id: &ToolId,
        local: LocalDeveloperToolPackage,
    ) -> Result<InstalledTool> {
        let _transaction = tool_state_guard(self.home)?;
        let tools = load_catalog_unlocked(self.home)?;
        let entry = tools
            .get(id)
            .ok_or_else(|| DistributionError::ToolNotInstalled { id: id.clone() })?;
        let active = entry.active.clone();
        let lock = read_tool_lock_unlocked(self.home, id)?;
        validate_active_pair(&active, &entry.rollback, &lock)?;
        if active.provenance != crate::ToolProvenance::LocalDeveloper {
            return Err(repair_mismatch(
                &active,
                "installed release is not a local developer package",
            ));
        }
        let candidate = ToolPackageCandidate::local(local)?;
        validate_local_repair_candidate(&active, &candidate)?;
        let shared_files = shared_digest_files(self.home, &tools, &active);
        let shared_directories = shared_digest_directories(self.home, &tools, &active);
        let transaction = begin_repair(self.home, &active)?;
        let digest_path = self.home.tools_store_dir().join(active.digest.to_string());
        let previous_path = quarantined_digest_path(self.home, id, &active.digest);

        let repair = ToolPackageStore::new(self.home)
            .prepare_candidate(candidate)
            .and_then(|package| {
                validate_repair_package(&active, &package)?;
                verify_package(self.home, &package)?;
                sync_package(self.home, &package)?;
                restore_shared_directories(&digest_path, &previous_path, &shared_directories)?;
                restore_shared_files(self.home, &digest_path, &previous_path, &shared_files)?;
                Ok(())
            });
        if let Err(original) = repair {
            if let Err(rollback) = rollback_repair(self.home, &transaction) {
                return Err(DistributionError::StateRollback {
                    original: Box::new(original),
                    rollback: Box::new(rollback),
                });
            }
            return Err(original);
        }
        commit_repair(self.home, transaction)?;
        tracing::info!(
            tool_id = %active.tool_id,
            version = %active.version,
            digest = %active.digest,
            "local developer tool bytes repaired"
        );
        Ok(active)
    }
}

fn validate_local_repair_candidate(
    active: &InstalledTool,
    candidate: &ToolPackageCandidate,
) -> Result<()> {
    let mismatch = if candidate.tool_id != active.tool_id {
        Some("tool identity differs")
    } else if candidate.tool_name != active.tool_name {
        Some("tool name differs")
    } else if candidate.version != active.version {
        Some("version differs")
    } else if candidate.platform != active.platform {
        Some("platform differs")
    } else if candidate.digest != active.digest {
        Some("artifact digest differs")
    } else if candidate.length != active.length {
        Some("artifact length differs")
    } else if candidate.target_path != active.target_path {
        Some("target path differs")
    } else if candidate.args != active.args {
        Some("launch arguments differ")
    } else {
        None
    };
    match mismatch {
        Some(reason) => Err(repair_mismatch(active, reason)),
        None => Ok(()),
    }
}

fn validate_repair_resolution(
    active: &InstalledTool,
    resolved: &ResolvedTrustedToolArtifact,
) -> Result<()> {
    let release = resolved.release();
    let artifact = resolved.artifact();
    let mismatch = if release.tool_id() != &active.tool_id {
        Some("tool identity differs")
    } else if release.tool_name() != active.tool_name {
        Some("tool name differs")
    } else if release.version() != &active.version {
        Some("version differs")
    } else if artifact.platform() != &active.platform {
        Some("platform differs")
    } else if resolved.digest() != &active.digest {
        Some("artifact digest differs")
    } else if resolved.length() != active.length {
        Some("artifact length differs")
    } else if artifact.target_path() != &active.target_path {
        Some("target path differs")
    } else if artifact.launch().args() != active.args {
        Some("launch arguments differ")
    } else {
        None
    };
    match mismatch {
        Some(reason) => Err(repair_mismatch(active, reason)),
        None => Ok(()),
    }
}

fn validate_repair_package(active: &InstalledTool, package: &VerifiedToolPackage) -> Result<()> {
    let mismatch = if package.tool_id != active.tool_id {
        Some("tool identity differs after materialization")
    } else if package.tool_name != active.tool_name {
        Some("tool name differs after materialization")
    } else if package.version != active.version {
        Some("version differs after materialization")
    } else if package.platform != active.platform {
        Some("platform differs after materialization")
    } else if package.digest != active.digest {
        Some("artifact digest differs after materialization")
    } else if package.length != active.length {
        Some("artifact length differs after materialization")
    } else if package.target_path != active.target_path {
        Some("target path differs after materialization")
    } else if package.store_path != active.store_path {
        Some("installed launch path differs")
    } else if package.package_root != active.package_root {
        Some("installed package root differs")
    } else if package.args != active.args {
        Some("launch arguments differ after materialization")
    } else if package.files != active.files {
        Some("installed file manifest differs")
    } else if package.directories != active.directories {
        Some("installed directory manifest differs")
    } else {
        None
    };
    match mismatch {
        Some(reason) => Err(repair_mismatch(active, reason)),
        None => Ok(()),
    }
}

fn shared_digest_files(
    home: &MorphirHome,
    tools: &std::collections::BTreeMap<ToolId, super::catalog::ToolCatalogEntry>,
    active: &InstalledTool,
) -> Vec<PathBuf> {
    let digest_path = home.tools_store_dir().join(active.digest.to_string());
    let mut files = tools
        .values()
        .flat_map(|entry| std::iter::once(&entry.active).chain(entry.rollback.iter()))
        .filter(|installed| {
            installed.tool_id != active.tool_id
                || installed.version != active.version
                || installed.digest != active.digest
        })
        .flat_map(|installed| installed.files.iter())
        .filter_map(|file| {
            home.root()
                .join(file.path.as_path())
                .strip_prefix(&digest_path)
                .ok()
                .map(Path::to_path_buf)
        })
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    files
}

fn shared_digest_directories(
    home: &MorphirHome,
    tools: &std::collections::BTreeMap<ToolId, super::catalog::ToolCatalogEntry>,
    active: &InstalledTool,
) -> Vec<PathBuf> {
    let digest_path = home.tools_store_dir().join(active.digest.to_string());
    let mut directories = tools
        .values()
        .flat_map(|entry| std::iter::once(&entry.active).chain(entry.rollback.iter()))
        .filter(|installed| {
            installed.tool_id != active.tool_id
                || installed.version != active.version
                || installed.digest != active.digest
        })
        .flat_map(|installed| installed.directories.iter())
        .filter_map(|directory| {
            home.root()
                .join(directory.as_path())
                .strip_prefix(&digest_path)
                .ok()
                .map(Path::to_path_buf)
        })
        .collect::<Vec<_>>();
    directories.sort();
    directories.dedup();
    directories
}

fn restore_shared_directories(
    digest_path: &Path,
    previous_path: &Path,
    directories: &[PathBuf],
) -> Result<()> {
    for relative in directories {
        let source_path = previous_path.join(relative);
        let metadata = match fs::symlink_metadata(&source_path) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(DistributionError::Io {
                    path: source_path,
                    source,
                });
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        create_dir_all_durable(&digest_path.join(relative))?;
    }
    Ok(())
}

fn restore_shared_files(
    home: &MorphirHome,
    digest_path: &Path,
    previous_path: &Path,
    files: &[PathBuf],
) -> Result<()> {
    for relative in files {
        let source = previous_path.join(relative);
        let destination = digest_path.join(relative);
        if destination.exists() || !source.exists() {
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| DistributionError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::copy(&source, &destination).map_err(|source| DistributionError::Io {
            path: destination.clone(),
            source,
        })?;
        sync_installed_file(home, &destination)?;
    }
    Ok(())
}

fn repair_mismatch(active: &InstalledTool, reason: &'static str) -> DistributionError {
    DistributionError::ToolRepairMismatch {
        id: active.tool_id.clone(),
        version: active.version.clone(),
        reason,
    }
}

#[cfg(test)]
pub(super) fn simulate_interrupted_repair(
    home: &MorphirHome,
    active: &InstalledTool,
) -> Result<()> {
    let _transaction = begin_repair(home, active)?;
    Ok(())
}

/// Atomically activate the most recently retained release for one installed tool.
///
/// Inspect the validated installed snapshot before offering rollback, then confirm
/// that the prior active release became the retained rollback candidate:
///
/// ```no_run
/// use morphir_common::home::MorphirHome;
/// use morphir_distribution::{ToolId, list_installed_tools, rollback_tool};
///
/// # fn rollback_desktop() -> Result<(), Box<dyn std::error::Error>> {
/// let home = MorphirHome::resolve()?;
/// let id = ToolId::parse("desktop")?;
/// let installed = list_installed_tools(&home)?;
/// if let Some(snapshot) = installed.iter().find(|tool| tool.active().tool_id() == &id)
///     && let Some(candidate) = snapshot.rollback().first()
/// {
///     let previous_version = snapshot.active().version().clone();
///     let candidate_version = candidate.version().clone();
///     let restored = rollback_tool(&home, &id)?;
///     assert_eq!(restored.version(), &candidate_version);
///
///     let after = list_installed_tools(&home)?;
///     let snapshot = after.iter().find(|tool| tool.active().tool_id() == &id).unwrap();
///     assert_eq!(snapshot.active().version(), &candidate_version);
///     assert_eq!(snapshot.rollback()[0].version(), &previous_version);
/// }
/// # Ok(())
/// # }
/// ```
#[tracing::instrument(
    name = "morphir.tool.rollback",
    skip(home),
    fields(tool_id = %id),
    err
)]
pub fn rollback_tool(home: &MorphirHome, id: &ToolId) -> Result<InstalledTool> {
    rollback_with_writer(home, id, &FilesystemStateWriter)
}

pub(super) fn rollback_with_writer(
    home: &MorphirHome,
    id: &ToolId,
    writer: &impl StateWriter,
) -> Result<InstalledTool> {
    let _transaction = tool_state_guard(home)?;
    let mut tools = load_catalog_unlocked(home)?;
    let mut entry = tools
        .remove(id)
        .ok_or_else(|| DistributionError::ToolNotInstalled { id: id.clone() })?;
    let current_lock = read_tool_lock_unlocked(home, id)?;
    validate_active_pair(&entry.active, &entry.rollback, &current_lock)?;
    if entry.rollback.is_empty() {
        return Err(DistributionError::NoToolRollback { id: id.clone() });
    }
    let next = entry.rollback.remove(0);
    verify_installed(
        home,
        next.store_path.as_path(),
        next.package_root
            .as_ref()
            .map(RelativeArtifactPath::as_path),
        &next.files,
        &next.directories,
    )?;
    let previous = entry.active;
    entry.active = next.clone();
    entry.rollback.insert(0, previous);
    let lock = ToolLock::from_installed(&next, &entry.rollback);
    tools.insert(id.clone(), entry);
    let stored = ToolCatalogFile {
        schema_version: TOOL_CATALOG_SCHEMA_VERSION,
        tools: tools.into_values().collect(),
    };
    commit_state_pair(
        &tool_lock_path(home, id),
        &encode_json(&lock)?,
        &home.tools_catalog_file(),
        &encode_json(&stored)?,
        writer,
    )?;
    tracing::info!(
        tool_id = %next.tool_id,
        version = %next.version,
        "tool rollback committed"
    );
    Ok(next)
}

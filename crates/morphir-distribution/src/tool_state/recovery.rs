//! Exact-release repair and explicit rollback operations.

use super::catalog::{
    InstalledTool, TOOL_CATALOG_SCHEMA_VERSION, ToolCatalogFile, ToolLock, load_catalog_unlocked,
    read_tool_lock_unlocked, tool_lock_path, tool_state_guard, validate_active_pair,
};
use super::package::{ToolPackageStore, VerifiedToolPackage};
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
        let shared_package_roots = shared_digest_package_roots(self.home, &tools, &active);
        validate_repair_resolution(&active, &resolved)?;
        let transaction = begin_repair(self.home, &active)?;
        let digest_path = self.home.tools_store_dir().join(active.digest.to_string());
        let previous_path = quarantined_digest_path(self.home, id, &active.digest);

        let repair = ToolPackageStore::new(self.home)
            .prepare(resolved, downloaded)
            .and_then(|package| {
                validate_repair_package(&active, &package)?;
                verify_package(self.home, &package)?;
                sync_package(self.home, &package)?;
                restore_shared_directories(&digest_path, &previous_path, &shared_package_roots)?;
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

fn shared_digest_package_roots(
    home: &MorphirHome,
    tools: &std::collections::BTreeMap<ToolId, super::catalog::ToolCatalogEntry>,
    active: &InstalledTool,
) -> Vec<PathBuf> {
    let digest_path = home.tools_store_dir().join(active.digest.to_string());
    let mut roots = tools
        .values()
        .flat_map(|entry| std::iter::once(&entry.active).chain(entry.rollback.iter()))
        .filter(|installed| {
            installed.tool_id != active.tool_id
                || installed.version != active.version
                || installed.digest != active.digest
        })
        .filter_map(|installed| installed.package_root.as_ref())
        .filter_map(|root| {
            home.root()
                .join(root.as_path())
                .strip_prefix(&digest_path)
                .ok()
                .map(Path::to_path_buf)
        })
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    roots
}

fn restore_shared_directories(
    digest_path: &Path,
    previous_path: &Path,
    package_roots: &[PathBuf],
) -> Result<()> {
    for relative_root in package_roots {
        restore_directory_tree(
            &previous_path.join(relative_root),
            &digest_path.join(relative_root),
        )?;
    }
    Ok(())
}

fn restore_directory_tree(source_root: &Path, destination_root: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(source_root) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(DistributionError::Io {
                path: source_root.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(());
    }
    create_dir_all_durable(destination_root)?;
    let mut pending = vec![(source_root.to_path_buf(), destination_root.to_path_buf())];
    while let Some((source_directory, destination_directory)) = pending.pop() {
        let entries = fs::read_dir(&source_directory).map_err(|source| DistributionError::Io {
            path: source_directory.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| DistributionError::Io {
                path: source_directory.clone(),
                source,
            })?;
            let file_type = entry.file_type().map_err(|source| DistributionError::Io {
                path: entry.path(),
                source,
            })?;
            if file_type.is_symlink() || !file_type.is_dir() {
                continue;
            }
            let destination = destination_directory.join(entry.file_name());
            create_dir_all_durable(&destination)?;
            pending.push((entry.path(), destination));
        }
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

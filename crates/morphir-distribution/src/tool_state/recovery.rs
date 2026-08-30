//! Exact-release repair and explicit rollback operations.

use super::catalog::{
    InstalledTool, TOOL_CATALOG_SCHEMA_VERSION, ToolCatalogFile, ToolLock, load_catalog_unlocked,
    read_tool_lock_unlocked, tool_lock_path, tool_state_guard, validate_active_pair,
};
use super::package::{ToolPackageStore, VerifiedToolPackage};
use super::verification::{verify_installed, verify_package};
use crate::state_io::{FilesystemStateWriter, StateWriter, commit_state_pair, encode_json};
use crate::{
    DistributionError, DownloadedToolArtifact, ResolvedTrustedToolArtifact, Result, ToolId,
};
use morphir_common::home::MorphirHome;
use std::fs;
use std::io;
use std::path::Path;

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
        let active = tools
            .get(id)
            .map(|entry| entry.active.clone())
            .ok_or_else(|| DistributionError::ToolNotInstalled { id: id.clone() })?;
        let lock = read_tool_lock_unlocked(self.home, id)?;
        validate_active_pair(&active, &lock)?;

        let temp_root = self.home.temp_dir();
        fs::create_dir_all(&temp_root).map_err(|source| DistributionError::Io {
            path: temp_root.clone(),
            source,
        })?;
        let quarantine = tempfile::Builder::new()
            .prefix("tool-repair-")
            .tempdir_in(&temp_root)
            .map_err(|source| DistributionError::Io {
                path: temp_root,
                source,
            })?;
        let digest_path = self.home.tools_store_dir().join(active.digest.to_string());
        let previous_path = quarantine.path().join("previous");
        let had_previous = move_to_quarantine(&digest_path, &previous_path)?;

        let repair = validate_repair_resolution(&active, &resolved)
            .and_then(|()| ToolPackageStore::new(self.home).prepare(resolved, downloaded))
            .and_then(|package| {
                validate_repair_package(&active, &package)?;
                verify_package(self.home, &package)?;
                Ok(())
            });

        if let Err(original) = repair {
            if let Err(rollback) = restore_quarantined_package(
                &digest_path,
                had_previous.then_some(previous_path.as_path()),
            ) {
                return Err(DistributionError::StateRollback {
                    original: Box::new(original),
                    rollback: Box::new(rollback),
                });
            }
            return Err(original);
        }

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

fn repair_mismatch(active: &InstalledTool, reason: &'static str) -> DistributionError {
    DistributionError::ToolRepairMismatch {
        id: active.tool_id.clone(),
        version: active.version.clone(),
        reason,
    }
}

fn move_to_quarantine(path: &Path, destination: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            fs::rename(path, destination).map_err(|source| DistributionError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            Ok(true)
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(DistributionError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn restore_quarantined_package(path: &Path, previous: Option<&Path>) -> Result<()> {
    remove_tool_store_entry(path)?;
    if let Some(previous) = previous {
        fs::rename(previous, path).map_err(|source| DistributionError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

fn remove_tool_store_entry(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_file() => {
            fs::remove_file(path).map_err(|source| DistributionError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
        Ok(_) => fs::remove_dir_all(path).map_err(|source| DistributionError::Io {
            path: path.to_path_buf(),
            source,
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(DistributionError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
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
    if entry.rollback.is_empty() {
        return Err(DistributionError::NoToolRollback { id: id.clone() });
    }
    let next = entry.rollback.remove(0);
    verify_installed(home, next.store_path.as_path(), &next.files)?;
    let previous = entry.active;
    entry.active = next.clone();
    entry.rollback.insert(0, previous);
    let lock = ToolLock::from_installed(&next);
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

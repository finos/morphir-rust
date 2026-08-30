//! Crash recovery for exact-release tool repairs.

use super::catalog::InstalledTool;
use crate::state_io::{
    atomic_write_json, create_dir_all_durable, read_json, remove_file, sync_parent_directory,
};
use crate::{DistributionError, Result, Sha256Digest, ToolId};
use morphir_common::home::MorphirHome;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const REPAIR_JOURNAL_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum RepairPhase {
    Prepared,
    Committed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepairJournal {
    schema_version: u32,
    tool_id: ToolId,
    digest: Sha256Digest,
    had_previous: bool,
    phase: RepairPhase,
}

pub(super) struct RepairTransaction {
    journal: RepairJournal,
}

trait RepairCleanup {
    fn discard_previous(&self, home: &MorphirHome, journal: &RepairJournal) -> Result<()>;

    fn finish_journal(&self, home: &MorphirHome, id: &ToolId) -> Result<()>;
}

struct FilesystemRepairCleanup;

impl RepairCleanup for FilesystemRepairCleanup {
    fn discard_previous(&self, home: &MorphirHome, journal: &RepairJournal) -> Result<()> {
        discard_previous(home, journal)
    }

    fn finish_journal(&self, home: &MorphirHome, id: &ToolId) -> Result<()> {
        finish_journal(home, id)
    }
}

pub(super) fn begin_repair(
    home: &MorphirHome,
    active: &InstalledTool,
) -> Result<RepairTransaction> {
    let journal = RepairJournal {
        schema_version: REPAIR_JOURNAL_SCHEMA_VERSION,
        tool_id: active.tool_id.clone(),
        digest: active.digest.clone(),
        had_previous: path_exists(&digest_path(home, &active.digest))?,
        phase: RepairPhase::Prepared,
    };
    let quarantine = quarantined_digest_path(home, &journal.tool_id, &journal.digest);
    remove_entry(&quarantine)?;
    atomic_write_json(&repair_journal_path(home, &journal.tool_id), &journal)?;
    if journal.had_previous {
        rename_durable(&digest_path(home, &journal.digest), &quarantine)?;
    }
    Ok(RepairTransaction { journal })
}

pub(super) fn rollback_repair(home: &MorphirHome, transaction: &RepairTransaction) -> Result<()> {
    restore_prepared(home, &transaction.journal)?;
    finish_journal(home, &transaction.journal.tool_id)
}

pub(super) fn commit_repair(home: &MorphirHome, transaction: RepairTransaction) -> Result<()> {
    commit_repair_with_cleanup(home, transaction, &FilesystemRepairCleanup)
}

fn commit_repair_with_cleanup(
    home: &MorphirHome,
    transaction: RepairTransaction,
    cleanup: &impl RepairCleanup,
) -> Result<()> {
    let committed = RepairJournal {
        phase: RepairPhase::Committed,
        ..transaction.journal
    };
    atomic_write_json(&repair_journal_path(home, &committed.tool_id), &committed)?;
    if let Err(error) = cleanup.discard_previous(home, &committed) {
        tracing::warn!(
            tool_id = %committed.tool_id,
            digest = %committed.digest,
            error = %error,
            "tool repair committed; quarantine cleanup deferred"
        );
        return Ok(());
    }
    if let Err(error) = cleanup.finish_journal(home, &committed.tool_id) {
        tracing::warn!(
            tool_id = %committed.tool_id,
            digest = %committed.digest,
            error = %error,
            "tool repair committed; journal cleanup deferred"
        );
    }
    Ok(())
}

pub(super) fn recover_tool_repairs(home: &MorphirHome) -> Result<()> {
    let directory = home.tools_locks_dir();
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(DistributionError::Io {
                path: directory,
                source,
            });
        }
    };
    for entry in entries {
        let entry = entry.map_err(|source| DistributionError::Io {
            path: directory.clone(),
            source,
        })?;
        let file_type = entry.file_type().map_err(|source| DistributionError::Io {
            path: entry.path(),
            source,
        })?;
        if !file_type.is_file() || file_type.is_symlink() {
            continue;
        }
        let Some(filename) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !filename.ends_with(".repair.json") {
            continue;
        }
        let journal: RepairJournal = read_json(&entry.path())?;
        if journal.schema_version != REPAIR_JOURNAL_SCHEMA_VERSION {
            return Err(DistributionError::UnsupportedStateSchema {
                kind: "tool repair transaction journal",
                version: journal.schema_version,
            });
        }
        match journal.phase {
            RepairPhase::Prepared => restore_prepared(home, &journal)?,
            RepairPhase::Committed => discard_previous(home, &journal)?,
        }
        finish_journal(home, &journal.tool_id)?;
    }
    Ok(())
}

pub(super) fn repair_journal_path(home: &MorphirHome, id: &ToolId) -> PathBuf {
    home.tools_locks_dir().join(format!("{id}.repair.json"))
}

fn restore_prepared(home: &MorphirHome, journal: &RepairJournal) -> Result<()> {
    let digest = digest_path(home, &journal.digest);
    let previous = quarantined_digest_path(home, &journal.tool_id, &journal.digest);
    if path_exists(&previous)? {
        remove_entry(&digest)?;
        rename_durable(&previous, &digest)?;
    } else if !journal.had_previous {
        remove_entry(&digest)?;
    }
    Ok(())
}

fn discard_previous(home: &MorphirHome, journal: &RepairJournal) -> Result<()> {
    remove_entry(&quarantined_digest_path(
        home,
        &journal.tool_id,
        &journal.digest,
    ))
}

fn finish_journal(home: &MorphirHome, id: &ToolId) -> Result<()> {
    remove_file(&repair_journal_path(home, id))
}

fn digest_path(home: &MorphirHome, digest: &Sha256Digest) -> PathBuf {
    home.tools_store_dir().join(digest.to_string())
}

pub(super) fn quarantined_digest_path(
    home: &MorphirHome,
    id: &ToolId,
    digest: &Sha256Digest,
) -> PathBuf {
    home.tools_store_dir()
        .join(format!(".repair-{id}-{digest}"))
}

fn path_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(DistributionError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn rename_durable(source: &Path, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .expect("repair quarantine destination has a parent");
    create_dir_all_durable(parent)?;
    fs::rename(source, destination).map_err(|error| DistributionError::Io {
        path: source.to_path_buf(),
        source: error,
    })?;
    sync_parent_directory(source)?;
    if source.parent() != destination.parent() {
        sync_parent_directory(destination)?;
    }
    Ok(())
}

fn remove_entry(path: &Path) -> Result<()> {
    let removed = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_file() => {
            fs::remove_file(path)
        }
        Ok(_) => fs::remove_dir_all(path),
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(DistributionError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    removed.map_err(|source| DistributionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    sync_parent_directory(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    enum CleanupFailure {
        Quarantine,
        Journal,
    }

    struct FailingCleanup(CleanupFailure);

    impl RepairCleanup for FailingCleanup {
        fn discard_previous(&self, home: &MorphirHome, journal: &RepairJournal) -> Result<()> {
            if matches!(self.0, CleanupFailure::Quarantine) {
                return Err(injected_cleanup_error(quarantined_digest_path(
                    home,
                    &journal.tool_id,
                    &journal.digest,
                )));
            }
            discard_previous(home, journal)
        }

        fn finish_journal(&self, home: &MorphirHome, id: &ToolId) -> Result<()> {
            if matches!(self.0, CleanupFailure::Journal) {
                return Err(injected_cleanup_error(repair_journal_path(home, id)));
            }
            finish_journal(home, id)
        }
    }

    #[test]
    fn committed_repair_defers_quarantine_cleanup_failure() {
        assert_committed_cleanup_is_recovered(CleanupFailure::Quarantine);
    }

    #[test]
    fn committed_repair_defers_journal_cleanup_failure() {
        assert_committed_cleanup_is_recovered(CleanupFailure::Journal);
    }

    fn assert_committed_cleanup_is_recovered(failure: CleanupFailure) {
        let root = tempfile::tempdir().unwrap();
        let home =
            MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
        let id = ToolId::parse("desktop").unwrap();
        let digest = Sha256Digest::of_bytes(b"desktop");
        let live = digest_path(&home, &digest);
        let quarantine = quarantined_digest_path(&home, &id, &digest);
        fs::create_dir_all(&live).unwrap();
        fs::write(live.join("replacement"), b"replacement").unwrap();
        fs::create_dir_all(&quarantine).unwrap();
        fs::write(quarantine.join("previous"), b"previous").unwrap();
        let transaction = RepairTransaction {
            journal: RepairJournal {
                schema_version: REPAIR_JOURNAL_SCHEMA_VERSION,
                tool_id: id.clone(),
                digest,
                had_previous: true,
                phase: RepairPhase::Prepared,
            },
        };

        commit_repair_with_cleanup(&home, transaction, &FailingCleanup(failure)).unwrap();

        let journal_path = repair_journal_path(&home, &id);
        let journal: RepairJournal = read_json(&journal_path).unwrap();
        assert_eq!(journal.phase, RepairPhase::Committed);
        assert!(live.join("replacement").exists());
        recover_tool_repairs(&home).unwrap();
        assert!(live.join("replacement").exists());
        assert!(!quarantine.exists());
        assert!(!journal_path.exists());
    }

    fn injected_cleanup_error(path: PathBuf) -> DistributionError {
        DistributionError::Io {
            path,
            source: io::Error::other("injected committed repair cleanup failure"),
        }
    }
}

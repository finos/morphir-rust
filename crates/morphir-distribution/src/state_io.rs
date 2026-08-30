//! Shared durable-state locking and atomic pair transactions.

use crate::{DistributionError, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const STATE_PAIR_JOURNAL_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StatePairJournal {
    schema_version: u32,
    next_lock: Option<Vec<u8>>,
    next_catalog: Option<Vec<u8>>,
}

#[derive(Debug)]
pub(crate) struct StateGuard {
    file: File,
}

impl StateGuard {
    pub(crate) fn acquire(path: &Path) -> Result<Self> {
        let parent = path.parent().expect("durable lock path has a parent");
        create_dir_all_durable(parent)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|source| DistributionError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        file.lock_exclusive()
            .map_err(|source| DistributionError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        Ok(Self { file })
    }
}

impl Drop for StateGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

pub(crate) trait StateWriter {
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<()>;

    fn remove(&self, path: &Path) -> Result<()> {
        remove_file(path)
    }

    fn cleanup_journal(&self, path: &Path) -> Result<()> {
        remove_file(path)
    }
}

pub(crate) struct FilesystemStateWriter;

impl StateWriter for FilesystemStateWriter {
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        atomic_write_bytes(path, bytes)
    }
}

pub(crate) fn commit_state_pair(
    lock_path: &Path,
    lock_bytes: &[u8],
    catalog_path: &Path,
    catalog_bytes: &[u8],
    writer: &impl StateWriter,
) -> Result<()> {
    let previous_lock = read_optional(lock_path)?;
    let previous_catalog = read_optional(catalog_path)?;
    let journal_path = state_pair_journal_path(lock_path);
    atomic_write_json(
        &journal_path,
        &StatePairJournal {
            schema_version: STATE_PAIR_JOURNAL_SCHEMA_VERSION,
            next_lock: Some(lock_bytes.to_vec()),
            next_catalog: Some(catalog_bytes.to_vec()),
        },
    )?;
    if let Err(original) = writer.write(lock_path, lock_bytes) {
        return rollback_state_error(
            original,
            &journal_path,
            lock_path,
            previous_lock.as_deref(),
            catalog_path,
            previous_catalog.as_deref(),
            writer,
        );
    }
    if let Err(original) = writer.write(catalog_path, catalog_bytes) {
        return rollback_state_error(
            original,
            &journal_path,
            lock_path,
            previous_lock.as_deref(),
            catalog_path,
            previous_catalog.as_deref(),
            writer,
        );
    }
    cleanup_committed_journal(writer, &journal_path);
    Ok(())
}

pub(crate) fn remove_state_pair(
    lock_path: &Path,
    catalog_path: &Path,
    catalog_bytes: &[u8],
    writer: &impl StateWriter,
) -> Result<()> {
    let previous_lock = read_optional(lock_path)?;
    let previous_catalog = read_optional(catalog_path)?;
    let journal_path = state_pair_journal_path(lock_path);
    atomic_write_json(
        &journal_path,
        &StatePairJournal {
            schema_version: STATE_PAIR_JOURNAL_SCHEMA_VERSION,
            next_lock: None,
            next_catalog: Some(catalog_bytes.to_vec()),
        },
    )?;
    if let Err(original) = writer.remove(lock_path) {
        return rollback_state_error(
            original,
            &journal_path,
            lock_path,
            previous_lock.as_deref(),
            catalog_path,
            previous_catalog.as_deref(),
            writer,
        );
    }
    if let Err(original) = writer.write(catalog_path, catalog_bytes) {
        return rollback_state_error(
            original,
            &journal_path,
            lock_path,
            previous_lock.as_deref(),
            catalog_path,
            previous_catalog.as_deref(),
            writer,
        );
    }
    cleanup_committed_journal(writer, &journal_path);
    Ok(())
}

fn cleanup_committed_journal(writer: &impl StateWriter, journal_path: &Path) {
    if let Err(error) = writer.cleanup_journal(journal_path) {
        tracing::warn!(
            path = %journal_path.display(),
            error = %error,
            "state update committed; transaction journal cleanup deferred"
        );
    }
}

fn rollback_state_error(
    original: DistributionError,
    journal_path: &Path,
    lock_path: &Path,
    lock_bytes: Option<&[u8]>,
    catalog_path: &Path,
    catalog_bytes: Option<&[u8]>,
    writer: &impl StateWriter,
) -> Result<()> {
    if let Err(rollback) = atomic_write_json(
        journal_path,
        &StatePairJournal {
            schema_version: STATE_PAIR_JOURNAL_SCHEMA_VERSION,
            next_lock: lock_bytes.map(ToOwned::to_owned),
            next_catalog: catalog_bytes.map(ToOwned::to_owned),
        },
    ) {
        return Err(DistributionError::StateRollback {
            original: Box::new(original),
            rollback: Box::new(rollback),
        });
    }
    match restore_state_pair(lock_path, lock_bytes, catalog_path, catalog_bytes) {
        Ok(()) => match writer.cleanup_journal(journal_path) {
            Ok(()) => Err(original),
            Err(rollback) => Err(DistributionError::StateRollback {
                original: Box::new(original),
                rollback: Box::new(rollback),
            }),
        },
        Err(rollback) => Err(DistributionError::StateRollback {
            original: Box::new(original),
            rollback: Box::new(rollback),
        }),
    }
}

pub(crate) fn recover_state_pairs(lock_directory: &Path, catalog_path: &Path) -> Result<()> {
    let entries = match fs::read_dir(lock_directory) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(DistributionError::Io {
                path: lock_directory.to_path_buf(),
                source,
            });
        }
    };
    for entry in entries {
        let entry = entry.map_err(|source| DistributionError::Io {
            path: lock_directory.to_path_buf(),
            source,
        })?;
        let file_type = entry.file_type().map_err(|source| DistributionError::Io {
            path: entry.path(),
            source,
        })?;
        if !file_type.is_file() || file_type.is_symlink() {
            continue;
        }
        let filename = entry.file_name();
        let Some(filename) = filename.to_str() else {
            continue;
        };
        let Some(lock_filename) = filename.strip_suffix(".transaction") else {
            continue;
        };
        if !lock_filename.ends_with(".json") {
            continue;
        }
        let journal_path = entry.path();
        let journal: StatePairJournal = read_json(&journal_path)?;
        if journal.schema_version != STATE_PAIR_JOURNAL_SCHEMA_VERSION {
            return Err(DistributionError::UnsupportedStateSchema {
                kind: "state pair transaction journal",
                version: journal.schema_version,
            });
        }
        let lock_path = lock_directory.join(lock_filename);
        restore_file(&lock_path, journal.next_lock.as_deref())?;
        restore_file(catalog_path, journal.next_catalog.as_deref())?;
        remove_file(&journal_path)?;
    }
    Ok(())
}

fn state_pair_journal_path(lock_path: &Path) -> PathBuf {
    let mut filename = lock_path
        .file_name()
        .expect("exact state lock has a filename")
        .to_os_string();
    filename.push(".transaction");
    lock_path.with_file_name(filename)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(DistributionError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn restore_state_pair(
    lock_path: &Path,
    lock_bytes: Option<&[u8]>,
    catalog_path: &Path,
    catalog_bytes: Option<&[u8]>,
) -> Result<()> {
    let lock_result = restore_file(lock_path, lock_bytes);
    let catalog_result = restore_file(catalog_path, catalog_bytes);
    lock_result.and(catalog_result)
}

fn restore_file(path: &Path, bytes: Option<&[u8]>) -> Result<()> {
    if let Some(bytes) = bytes {
        return atomic_write_bytes(path, bytes);
    }
    remove_file(path)
}

pub(crate) fn remove_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => sync_parent_directory(path),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(DistributionError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

pub(crate) fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = read_state_bytes(path)?;
    decode_state(path, &bytes)
}

pub(crate) fn read_state_bytes(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).map_err(|source| DistributionError::Io {
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn decode_state<T: for<'de> Deserialize<'de>>(path: &Path, bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|source| DistributionError::InvalidState {
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn atomic_write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    atomic_write_bytes(path, &encode_json(value)?)
}

pub(crate) fn encode_json(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(DistributionError::StateEncoding)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(crate) fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().expect("durable state path has a parent");
    create_dir_all_durable(parent)?;
    let mut staged = tempfile::Builder::new()
        .prefix(".stage-")
        .tempfile_in(parent)
        .map_err(|source| DistributionError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    staged
        .as_file_mut()
        .write_all(bytes)
        .and_then(|()| staged.as_file_mut().flush())
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|source| DistributionError::Io {
            path: staged.path().to_path_buf(),
            source,
        })?;
    staged
        .persist(path)
        .map_err(|error| DistributionError::Io {
            path: path.to_path_buf(),
            source: error.error,
        })?;
    sync_parent_directory(path)
}

pub(crate) fn create_dir_all_durable(path: &Path) -> Result<()> {
    let missing = path
        .ancestors()
        .take_while(|ancestor| !ancestor.exists())
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    fs::create_dir_all(path).map_err(|source| DistributionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    for created in missing.iter().rev() {
        sync_parent_directory(created)?;
    }
    Ok(())
}

pub(crate) fn sync_parent_directory(path: &Path) -> Result<()> {
    sync_directory(parent_directory(path))
}

fn parent_directory(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if parent.as_os_str().is_empty() => Path::new("."),
        Some(parent) => parent,
        None => path,
    }
}

#[cfg(unix)]
fn sync_directory(directory: &Path) -> Result<()> {
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|source| DistributionError::Io {
            path: directory.to_path_buf(),
            source,
        })
}

#[cfg(not(unix))]
fn sync_directory(_directory: &Path) -> Result<()> {
    // Windows does not expose a portable directory-sync operation through std.
    // Atomic replacement still flushes the staged file before the rename.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_component_paths_sync_through_the_current_directory() {
        assert_eq!(parent_directory(Path::new("mh")), Path::new("."));
    }

    struct FailingCatalogAndJournalCleanup {
        catalog_path: PathBuf,
    }

    struct FailingJournalCleanup;

    impl StateWriter for FailingCatalogAndJournalCleanup {
        fn write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
            atomic_write_bytes(path, bytes)?;
            if path == self.catalog_path {
                return Err(DistributionError::Io {
                    path: path.to_path_buf(),
                    source: std::io::Error::other("injected catalog failure"),
                });
            }
            Ok(())
        }

        fn cleanup_journal(&self, path: &Path) -> Result<()> {
            Err(DistributionError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::other("injected journal cleanup failure"),
            })
        }
    }

    impl StateWriter for FailingJournalCleanup {
        fn write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
            atomic_write_bytes(path, bytes)
        }

        fn cleanup_journal(&self, path: &Path) -> Result<()> {
            Err(DistributionError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::other("injected journal cleanup failure"),
            })
        }
    }

    #[test]
    fn recovery_completes_an_interrupted_state_pair_commit() {
        let root = tempfile::tempdir().unwrap();
        let locks = root.path().join("locks");
        let lock = locks.join("desktop.json");
        let catalog = root.path().join("catalog.json");
        atomic_write_bytes(&lock, b"old lock").unwrap();
        atomic_write_bytes(&catalog, b"old catalog").unwrap();
        let journal = state_pair_journal_path(&lock);
        atomic_write_json(
            &journal,
            &StatePairJournal {
                schema_version: STATE_PAIR_JOURNAL_SCHEMA_VERSION,
                next_lock: Some(b"new lock".to_vec()),
                next_catalog: Some(b"new catalog".to_vec()),
            },
        )
        .unwrap();
        atomic_write_bytes(&lock, b"new lock").unwrap();

        recover_state_pairs(&locks, &catalog).unwrap();

        assert_eq!(fs::read(lock).unwrap(), b"new lock");
        assert_eq!(fs::read(catalog).unwrap(), b"new catalog");
        assert!(!journal.exists());
    }

    #[test]
    fn failed_rollback_cleanup_can_only_replay_the_restored_state() {
        let root = tempfile::tempdir().unwrap();
        let locks = root.path().join("locks");
        let lock = locks.join("desktop.json");
        let catalog = root.path().join("catalog.json");
        atomic_write_bytes(&lock, b"old lock").unwrap();
        atomic_write_bytes(&catalog, b"old catalog").unwrap();
        let writer = FailingCatalogAndJournalCleanup {
            catalog_path: catalog.clone(),
        };

        assert!(matches!(
            commit_state_pair(&lock, b"new lock", &catalog, b"new catalog", &writer),
            Err(DistributionError::StateRollback { .. })
        ));
        assert_eq!(fs::read(&lock).unwrap(), b"old lock");
        assert_eq!(fs::read(&catalog).unwrap(), b"old catalog");

        recover_state_pairs(&locks, &catalog).unwrap();

        assert_eq!(fs::read(lock).unwrap(), b"old lock");
        assert_eq!(fs::read(catalog).unwrap(), b"old catalog");
    }

    #[test]
    fn committed_state_pair_succeeds_when_journal_cleanup_is_deferred() {
        let root = tempfile::tempdir().unwrap();
        let locks = root.path().join("locks");
        let lock = locks.join("desktop.json");
        let catalog = root.path().join("catalog.json");
        atomic_write_bytes(&lock, b"old lock").unwrap();
        atomic_write_bytes(&catalog, b"old catalog").unwrap();
        let journal = state_pair_journal_path(&lock);

        commit_state_pair(
            &lock,
            b"new lock",
            &catalog,
            b"new catalog",
            &FailingJournalCleanup,
        )
        .unwrap();

        assert_eq!(fs::read(&lock).unwrap(), b"new lock");
        assert_eq!(fs::read(&catalog).unwrap(), b"new catalog");
        assert!(journal.exists());
        recover_state_pairs(&locks, &catalog).unwrap();
        assert_eq!(fs::read(lock).unwrap(), b"new lock");
        assert_eq!(fs::read(catalog).unwrap(), b"new catalog");
        assert!(!journal.exists());
    }

    #[test]
    fn removed_state_pair_succeeds_when_journal_cleanup_is_deferred() {
        let root = tempfile::tempdir().unwrap();
        let locks = root.path().join("locks");
        let lock = locks.join("desktop.json");
        let catalog = root.path().join("catalog.json");
        atomic_write_bytes(&lock, b"old lock").unwrap();
        atomic_write_bytes(&catalog, b"old catalog").unwrap();
        let journal = state_pair_journal_path(&lock);

        remove_state_pair(&lock, &catalog, b"new catalog", &FailingJournalCleanup).unwrap();

        assert!(!lock.exists());
        assert_eq!(fs::read(&catalog).unwrap(), b"new catalog");
        assert!(journal.exists());
        recover_state_pairs(&locks, &catalog).unwrap();
        assert!(!lock.exists());
        assert_eq!(fs::read(catalog).unwrap(), b"new catalog");
        assert!(!journal.exists());
    }
}

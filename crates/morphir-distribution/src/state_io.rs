//! Shared durable-state locking and atomic pair transactions.

use crate::{DistributionError, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

pub(crate) struct StateGuard {
    file: File,
}

impl StateGuard {
    pub(crate) fn acquire(path: &Path) -> Result<Self> {
        let parent = path.parent().expect("durable lock path has a parent");
        fs::create_dir_all(parent).map_err(|source| DistributionError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
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
    writer.write(lock_path, lock_bytes)?;
    if let Err(original) = writer.write(catalog_path, catalog_bytes) {
        return rollback_state_error(
            original,
            lock_path,
            previous_lock.as_deref(),
            catalog_path,
            previous_catalog.as_deref(),
        );
    }
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
    if let Err(original) = writer.remove(lock_path) {
        return rollback_state_error(
            original,
            lock_path,
            previous_lock.as_deref(),
            catalog_path,
            previous_catalog.as_deref(),
        );
    }
    if let Err(original) = writer.write(catalog_path, catalog_bytes) {
        return rollback_state_error(
            original,
            lock_path,
            previous_lock.as_deref(),
            catalog_path,
            previous_catalog.as_deref(),
        );
    }
    Ok(())
}

fn rollback_state_error(
    original: DistributionError,
    lock_path: &Path,
    lock_bytes: Option<&[u8]>,
    catalog_path: &Path,
    catalog_bytes: Option<&[u8]>,
) -> Result<()> {
    match restore_state_pair(lock_path, lock_bytes, catalog_path, catalog_bytes) {
        Ok(()) => Err(original),
        Err(rollback) => Err(DistributionError::StateRollback {
            original: Box::new(original),
            rollback: Box::new(rollback),
        }),
    }
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
        Ok(()) => Ok(()),
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
    fs::create_dir_all(parent).map_err(|source| DistributionError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
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
    Ok(())
}

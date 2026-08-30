use super::{CacheExecutionError, cap_is_link_like, crosses_filesystem_boundary, io_error};
use crate::cache_maintenance::fingerprint::{finish, metadata_fingerprint, metadata_hasher};
use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use std::path::Path;

pub(super) struct TreeObservation {
    pub(super) bytes: u64,
    pub(super) fingerprint: u64,
}

pub(super) fn observe_tree(
    parent: &Dir,
    name: &Path,
    path: &Path,
) -> Result<TreeObservation, CacheExecutionError> {
    let metadata = parent
        .symlink_metadata(name)
        .map_err(|source| io_error(path, source))?;
    if cap_is_link_like(&metadata) || (!metadata.is_dir() && !metadata.is_file()) {
        return Err(CacheExecutionError::UnsafeMaintenancePath {
            path: path.to_path_buf(),
        });
    }
    if metadata.is_file() {
        return Ok(TreeObservation {
            bytes: metadata.len(),
            fingerprint: metadata_fingerprint(&metadata),
        });
    }
    if crosses_filesystem_boundary(parent, &metadata, path)? {
        return Err(CacheExecutionError::UnsafeMaintenancePath {
            path: path.to_path_buf(),
        });
    }
    let hasher = metadata_hasher(&metadata);
    let directory = parent
        .open_dir_nofollow(name)
        .map_err(|source| io_error(path, source))?;
    let mut entries = directory
        .entries()
        .map_err(|source| io_error(path, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| io_error(path, source))?;
    entries.sort_by_key(cap_std::fs::DirEntry::file_name);
    let mut bytes = 0_u64;
    let mut children = Vec::with_capacity(entries.len());
    for entry in entries {
        let child = entry.file_name();
        let observation = observe_tree(&directory, Path::new(&child), &path.join(&child))?;
        bytes = bytes
            .checked_add(observation.bytes)
            .ok_or(CacheExecutionError::ByteCountOverflow)?;
        children.push((child, observation.fingerprint));
    }
    Ok(TreeObservation {
        bytes,
        fingerprint: finish(hasher, &children),
    })
}

pub(super) fn remove_tree(
    parent: &Dir,
    name: &Path,
    path: &Path,
) -> Result<(), CacheExecutionError> {
    let metadata = parent
        .symlink_metadata(name)
        .map_err(|source| io_error(path, source))?;
    if metadata.is_dir() && !cap_is_link_like(&metadata) {
        if crosses_filesystem_boundary(parent, &metadata, path)? {
            return Err(CacheExecutionError::UnsafeMaintenancePath {
                path: path.to_path_buf(),
            });
        }
        let directory = parent
            .open_dir_nofollow(name)
            .map_err(|source| io_error(path, source))?;
        let mut entries = directory
            .entries()
            .map_err(|source| io_error(path, source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| io_error(path, source))?;
        entries.sort_by_key(cap_std::fs::DirEntry::file_name);
        for entry in entries {
            let child = entry.file_name();
            remove_tree(&directory, Path::new(&child), &path.join(&child))?;
        }
        drop(directory);
        parent
            .remove_dir(name)
            .map_err(|source| io_error(path, source))
    } else if metadata.is_file() || cap_is_link_like(&metadata) {
        parent
            .remove_file(name)
            .map_err(|source| io_error(path, source))
    } else {
        Err(CacheExecutionError::UnsafeMaintenancePath {
            path: path.to_path_buf(),
        })
    }
}

use super::{CacheInventoryError, InventoryWalk, io_error, is_link_like, native};
use crate::cache_maintenance::fingerprint::{finish, metadata_fingerprint, metadata_hasher};
use cap_fs_ext::DirExt;
use cap_std::fs::{Dir, DirEntry, Metadata};
use std::path::Path;

pub(super) struct MeasuredEntry {
    pub(super) bytes: u64,
    pub(super) safe: bool,
    pub(super) fingerprint: u64,
}

impl InventoryWalk<'_> {
    pub(super) fn measure(
        &mut self,
        parent: &Dir,
        entry: &DirEntry,
        path: &Path,
        metadata: &Metadata,
        depth: usize,
    ) -> Result<MeasuredEntry, CacheInventoryError> {
        let hasher = metadata_hasher(metadata);
        if is_link_like(metadata) || native::crosses_filesystem_boundary(parent, metadata) {
            return Ok(MeasuredEntry::leaf(metadata, false));
        }
        if metadata.is_file() {
            return Ok(MeasuredEntry::leaf(metadata, true));
        }
        if !metadata.is_dir() {
            return Ok(MeasuredEntry::leaf(metadata, false));
        }

        self.check_depth(depth)?;
        let directory = match parent.open_dir_nofollow(entry.file_name()) {
            Ok(directory) => directory,
            Err(_) => return Ok(MeasuredEntry::leaf(metadata, false)),
        };
        let mut bytes = 0_u64;
        let mut safe = true;
        let mut children = Vec::new();
        for child in self.read_children(&directory, path)? {
            let child_path = path.join(child.file_name());
            let child_metadata = directory
                .symlink_metadata(child.file_name())
                .map_err(|source| io_error(&child_path, source))?;
            let measured =
                self.measure(&directory, &child, &child_path, &child_metadata, depth + 1)?;
            bytes = bytes
                .checked_add(measured.bytes)
                .ok_or(CacheInventoryError::ByteCountOverflow)?;
            safe &= measured.safe;
            children.push((child.file_name(), measured.fingerprint));
        }
        Ok(MeasuredEntry {
            bytes,
            safe,
            fingerprint: finish(hasher, &children),
        })
    }
}

impl MeasuredEntry {
    fn leaf(metadata: &Metadata, safe: bool) -> Self {
        Self {
            bytes: metadata.len(),
            safe,
            fingerprint: metadata_fingerprint(metadata),
        }
    }
}

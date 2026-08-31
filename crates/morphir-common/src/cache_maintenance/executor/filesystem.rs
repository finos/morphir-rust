mod observation;

use self::observation::{observe_tree, remove_tree};
use super::{CacheExecutionError, CacheExecutionLimits};
use crate::home::MorphirHome;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
#[cfg(unix)]
use cap_fs_ext::{OpenOptionsMaybeDirExt, OpenOptionsSyncExt};
use cap_std::ambient_authority;
#[cfg(windows)]
use cap_std::fs::MetadataExt as CapMetadataExt;
use cap_std::fs::{Dir, Metadata as CapMetadata, OpenOptions as CapOpenOptions};
use fs2::FileExt;
use same_file::Handle;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use tracing::debug;
use uuid::Uuid;

pub(super) fn remove_revalidated_entry(
    target: RemovalTarget<'_>,
    trash_run: &TrashRun,
    index: usize,
) -> Result<RemovalOutcome, CacheExecutionError> {
    remove_revalidated_entry_with_hook(target, trash_run, index, || {})
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RemovalOutcome {
    Removed,
    Changed,
}

pub(super) struct RemovalTarget<'a> {
    pub(super) home: &'a MorphirHome,
    pub(super) home_dir: &'a Dir,
    pub(super) namespace: &'a str,
    pub(super) relative: &'a str,
    pub(super) expected: &'a Handle,
    pub(super) expected_bytes: u64,
    pub(super) expected_fingerprint: u64,
}

fn remove_revalidated_entry_with_hook<F>(
    target: RemovalTarget<'_>,
    trash_run: &TrashRun,
    index: usize,
    before_rename: F,
) -> Result<RemovalOutcome, CacheExecutionError>
where
    F: FnOnce(),
{
    let (parent, leaf, source) = open_removal_parent(
        target.home,
        target.home_dir,
        target.namespace,
        target.relative,
    )?;
    let source_object = pin_object(&parent, &leaf, &source)?;
    if source_object.handle != *target.expected {
        return Ok(RemovalOutcome::Changed);
    }
    before_rename();
    let staging_name = format!("{index:08x}");
    let staging_path = trash_run.path.join(&staging_name);
    parent
        .rename(&leaf, &trash_run.dir, &staging_name)
        .map_err(|error| io_error(&source, error))?;
    let staged_object = pin_object(&trash_run.dir, &staging_name, &staging_path)?;
    if source_object.handle != staged_object.handle || source_object.is_dir != staged_object.is_dir
    {
        return Err(CacheExecutionError::EntryChangedDuringRemoval {
            path: source,
            staged_path: staging_path,
        });
    }
    let observation = observe_tree(&trash_run.dir, Path::new(&staging_name), &staging_path)?;
    let confirmed_object = pin_object(&trash_run.dir, &staging_name, &staging_path)?;
    if staged_object.handle != confirmed_object.handle
        || observation.bytes != target.expected_bytes
        || observation.fingerprint != target.expected_fingerprint
    {
        return Err(CacheExecutionError::EntryChangedDuringRemoval {
            path: source,
            staged_path: staging_path,
        });
    }

    let verified_name = format!("verified-{staging_name}-{:016x}", target.expected_bytes);
    let verified_path = trash_run.path.join(&verified_name);
    trash_run
        .dir
        .rename(&staging_name, &trash_run.dir, &verified_name)
        .map_err(|source| io_error(&staging_path, source))?;
    if staged_object.is_dir {
        remove_tree(&trash_run.dir, Path::new(&verified_name), &verified_path)?;
    } else {
        trash_run
            .dir
            .remove_file(&verified_name)
            .map_err(|source| io_error(&verified_path, source))?;
    }
    Ok(RemovalOutcome::Removed)
}

struct PinnedObject {
    handle: Handle,
    is_dir: bool,
}

fn pin_object(parent: &Dir, name: &str, path: &Path) -> Result<PinnedObject, CacheExecutionError> {
    let metadata = parent
        .symlink_metadata(name)
        .map_err(|source| io_error(path, source))?;
    if cap_is_link_like(&metadata) || (!metadata.is_dir() && !metadata.is_file()) {
        return Err(CacheExecutionError::UnsafeMaintenancePath {
            path: path.to_path_buf(),
        });
    }
    let file = if metadata.is_dir() {
        parent
            .open_dir_nofollow(name)
            .map_err(|source| io_error(path, source))?
            .into_std_file()
    } else {
        let mut options = CapOpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        options.nonblock(true);
        parent
            .open_with(name, &options)
            .map_err(|source| io_error(path, source))?
            .into_std()
    };
    let opened = file.metadata().map_err(|source| io_error(path, source))?;
    if opened.is_dir() != metadata.is_dir() || opened.is_file() != metadata.is_file() {
        return Err(CacheExecutionError::UnsafeMaintenancePath {
            path: path.to_path_buf(),
        });
    }
    let handle = Handle::from_file(file).map_err(|source| io_error(path, source))?;
    Ok(PinnedObject {
        handle,
        is_dir: opened.is_dir(),
    })
}

fn open_removal_parent(
    home: &MorphirHome,
    home_dir: &Dir,
    namespace: &str,
    relative: &str,
) -> Result<(Dir, String, PathBuf), CacheExecutionError> {
    let cache_path = home.cache_dir();
    let cache_dir = home_dir
        .open_dir_nofollow("cache")
        .map_err(|source| io_error(&cache_path, source))?;
    let namespace_path = cache_path.join(namespace);
    let mut parent = cache_dir
        .open_dir_nofollow(namespace)
        .map_err(|source| io_error(&namespace_path, source))?;
    let mut display_parent = namespace_path;
    let mut segments = relative.split('/').peekable();
    let mut leaf = None;
    while let Some(segment) = segments.next() {
        if segments.peek().is_none() {
            leaf = Some(segment.to_owned());
            break;
        }
        display_parent.push(segment);
        parent = parent
            .open_dir_nofollow(segment)
            .map_err(|source| io_error(&display_parent, source))?;
    }
    let leaf = leaf.expect("validated cache identities contain at least one segment");
    let source = display_parent.join(&leaf);
    Ok((parent, leaf, source))
}

pub(super) struct MaintenanceTrash {
    dir: Dir,
    path: PathBuf,
}

pub(super) struct TrashRun {
    root: Dir,
    dir: Dir,
    name: String,
    path: PathBuf,
}

impl TrashRun {
    pub(super) fn finish(self) -> Result<(), CacheExecutionError> {
        let Self {
            root,
            dir,
            name,
            path,
        } = self;
        drop(dir);
        root.remove_dir(&name)
            .map_err(|source| io_error(&path, source))
    }
}

pub(super) fn open_maintenance_trash(
    home: &MorphirHome,
    home_dir: &Dir,
) -> Result<MaintenanceTrash, CacheExecutionError> {
    let temp_path = home.temp_dir();
    let temp_dir = open_or_create_directory(home_dir, "tmp", &temp_path)?;
    let path = home.maintenance_trash_dir();
    let dir = open_or_create_directory(&temp_dir, "maintenance-trash", &path)?;
    Ok(MaintenanceTrash { dir, path })
}

fn open_or_create_directory(
    parent: &Dir,
    name: &str,
    display_path: &Path,
) -> Result<Dir, CacheExecutionError> {
    match parent.open_dir_nofollow(name) {
        Ok(directory) => Ok(directory),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match parent.create_dir(name) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => return Err(io_error(display_path, source)),
            }
            parent
                .open_dir_nofollow(name)
                .map_err(|source| io_error(display_path, source))
        }
        Err(source) => Err(io_error(display_path, source)),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct RecoveryBudget {
    pub(super) removals: usize,
    pub(super) bytes: u64,
}

pub(super) fn sweep_existing_trash(
    trash: &MaintenanceTrash,
    limits: CacheExecutionLimits,
) -> Result<RecoveryBudget, CacheExecutionError> {
    let mut recovered = RecoveryBudget::default();
    let mut entries = trash
        .dir
        .entries()
        .map_err(|source| io_error(&trash.path, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| io_error(&trash.path, source))?;
    entries.sort_by_key(cap_std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        let Some(name_text) = name.to_str() else {
            continue;
        };
        if !is_trash_run_name(name_text) {
            continue;
        }
        let path = trash.path.join(&name);
        let metadata = trash
            .dir
            .symlink_metadata(&name)
            .map_err(|source| io_error(&path, source))?;
        if cap_is_link_like(&metadata) || !metadata.is_dir() {
            return Err(CacheExecutionError::UnsafeMaintenancePath { path });
        }
        let run = trash
            .dir
            .open_dir_nofollow(&name)
            .map_err(|source| io_error(&path, source))?;
        let verified_entries = verified_run_entries(&run, &path)?;
        for (verified_name, bytes) in verified_entries {
            let next_bytes = recovered
                .bytes
                .checked_add(bytes)
                .ok_or(CacheExecutionError::ByteCountOverflow)?;
            if recovered.removals == limits.max_removals || next_bytes > limits.max_bytes {
                return Ok(recovered);
            }
            remove_tree(&run, Path::new(&verified_name), &path.join(&verified_name))?;
            recovered.removals += 1;
            recovered.bytes = next_bytes;
        }
        drop(run);
        trash
            .dir
            .remove_dir(&name)
            .map_err(|source| io_error(&path, source))?;
        debug!(
            event = "cache_cleanup_trash_recovered",
            "recovered interrupted trash run"
        );
    }
    Ok(recovered)
}

fn verified_run_entries(
    directory: &Dir,
    display_path: &Path,
) -> Result<Vec<(String, u64)>, CacheExecutionError> {
    let mut entries = directory
        .entries()
        .map_err(|source| io_error(display_path, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| io_error(display_path, source))?;
    entries.sort_by_key(cap_std::fs::DirEntry::file_name);
    let mut verified = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = entry.file_name();
        let Some((name, bytes)) = name
            .to_str()
            .and_then(|name| verified_entry_bytes(name).map(|bytes| (name.to_owned(), bytes)))
        else {
            return Err(CacheExecutionError::UnsafeMaintenancePath {
                path: display_path.join(name),
            });
        };
        verified.push((name, bytes));
    }
    Ok(verified)
}

fn is_trash_run_name(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn verified_entry_bytes(value: &str) -> Option<u64> {
    let (index, bytes) = value.strip_prefix("verified-")?.split_once('-')?;
    (index.len() == 8 && index.bytes().all(is_lower_hex) && bytes.len() == 16)
        .then(|| u64::from_str_radix(bytes, 16).ok())
        .flatten()
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
}

#[cfg(unix)]
fn crosses_filesystem_boundary(
    parent: &Dir,
    child: &CapMetadata,
    path: &Path,
) -> Result<bool, CacheExecutionError> {
    use cap_std::fs::MetadataExt;

    let parent_metadata = parent
        .dir_metadata()
        .map_err(|source| io_error(path, source))?;
    Ok(parent_metadata.dev() != child.dev())
}

#[cfg(not(unix))]
fn crosses_filesystem_boundary(
    _parent: &Dir,
    _child: &CapMetadata,
    _path: &Path,
) -> Result<bool, CacheExecutionError> {
    Ok(false)
}

pub(super) fn create_trash_run(trash: &MaintenanceTrash) -> Result<TrashRun, CacheExecutionError> {
    for _ in 0..8 {
        let name = Uuid::new_v4().simple().to_string();
        let path = trash.path.join(&name);
        match trash.dir.create_dir(&name) {
            Ok(()) => {
                let dir = trash
                    .dir
                    .open_dir_nofollow(&name)
                    .map_err(|source| io_error(&path, source))?;
                let root = trash
                    .dir
                    .try_clone()
                    .map_err(|source| io_error(&trash.path, source))?;
                return Ok(TrashRun {
                    root,
                    dir,
                    name,
                    path,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(io_error(&path, source)),
        }
    }
    Err(io_error(
        &trash.path,
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a unique maintenance trash run",
        ),
    ))
}

pub(crate) struct MaintenanceGuard {
    file: File,
    home_dir: Dir,
}

impl MaintenanceGuard {
    pub(crate) fn acquire(home: &MorphirHome) -> Result<Self, CacheExecutionError> {
        let (file, home_dir) = open_maintenance_lock(home)?;
        let path = home.maintenance_lock_file();
        FileExt::lock_exclusive(&file).map_err(|source| io_error(&path, source))?;
        Ok(Self { file, home_dir })
    }

    pub(crate) fn home_dir(&self) -> &Dir {
        &self.home_dir
    }
}

impl Drop for MaintenanceGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// Shared lease held by cache producers and users while content may mutate.
///
/// Cleanup takes the corresponding exclusive lock. Holding this guard through
/// the lifetime of every writable cache handle prevents staged content from
/// changing between its final observation and deletion.
pub struct CacheMutationGuard {
    file: File,
}

impl CacheMutationGuard {
    /// Acquire the suite-wide shared cache mutation lease beneath Morphir Home.
    pub fn acquire(home: &MorphirHome) -> Result<Self, CacheExecutionError> {
        let (file, _home_dir) = open_maintenance_lock(home)?;
        let path = home.maintenance_lock_file();
        FileExt::lock_shared(&file).map_err(|source| io_error(&path, source))?;
        Ok(Self { file })
    }
}

impl Drop for CacheMutationGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn open_maintenance_lock(home: &MorphirHome) -> Result<(File, Dir), CacheExecutionError> {
    create_directory_tree_durably(home.root())?;
    let home_dir = Dir::open_ambient_dir(home.root(), ambient_authority())
        .map_err(|source| io_error(home.root(), source))?;
    let locks_path = home.locks_dir();
    let locks_dir = open_or_create_directory(&home_dir, "locks", &locks_path)?;
    let path = home.maintenance_lock_file();
    match locks_dir.symlink_metadata("maintenance.lock") {
        Ok(metadata) if cap_is_link_like(&metadata) || !metadata.is_file() => {
            return Err(CacheExecutionError::UnsafeMaintenancePath { path });
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(io_error(&path, source)),
    }
    let mut options = CapOpenOptions::new();
    options
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .follow(FollowSymlinks::No);
    let file = locks_dir
        .open_with("maintenance.lock", &options)
        .map(cap_std::fs::File::into_std)
        .map_err(|source| io_error(&path, source))?;
    Ok((file, home_dir))
}

fn create_directory_tree_durably(path: &Path) -> Result<(), CacheExecutionError> {
    create_directory_tree_durably_with(path, sync_ambient_directory)
}

fn create_directory_tree_durably_with<F>(
    path: &Path,
    mut sync_parent: F,
) -> Result<(), CacheExecutionError>
where
    F: FnMut(&Path) -> Result<(), CacheExecutionError>,
{
    let mut missing = Vec::new();
    let mut candidate = Some(path);
    while let Some(directory) = candidate {
        match fs::symlink_metadata(directory) {
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(directory.to_path_buf());
                candidate = nonempty_parent(directory);
            }
            Err(source) => return Err(io_error(directory, source)),
        }
    }

    fs::create_dir_all(path).map_err(|source| io_error(path, source))?;
    // Persist each new directory entry from the leaf toward the nearest
    // pre-existing ancestor so no durable parent depends on a lost child.
    for directory in missing {
        if let Some(parent) = directory_entry_parent(&directory) {
            sync_parent(parent)?;
        }
    }
    Ok(())
}

fn nonempty_parent(path: &Path) -> Option<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
}

fn directory_entry_parent(path: &Path) -> Option<&Path> {
    path.parent().map(|parent| {
        if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        }
    })
}

#[cfg(unix)]
fn sync_ambient_directory(path: &Path) -> Result<(), CacheExecutionError> {
    let directory = Dir::open_ambient_dir(path, ambient_authority())
        .map_err(|source| io_error(path, source))?;
    let mut options = CapOpenOptions::new();
    options
        .read(true)
        .maybe_dir(true)
        .follow(FollowSymlinks::No);
    directory
        .open_with(".", &options)
        .map(cap_std::fs::File::into_std)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error(path, source))
}

#[cfg(not(unix))]
fn sync_ambient_directory(_path: &Path) -> Result<(), CacheExecutionError> {
    Ok(())
}

fn io_error(path: &Path, source: io::Error) -> CacheExecutionError {
    CacheExecutionError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(windows)]
fn cap_is_link_like(metadata: &CapMetadata) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || CapMetadataExt::file_attributes(metadata) & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn cap_is_link_like(metadata: &CapMetadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(test)]
mod tests;

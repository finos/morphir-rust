use super::CacheExecutionError;
use crate::home::MorphirHome;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
#[cfg(windows)]
use cap_std::fs::MetadataExt as CapMetadataExt;
use cap_std::fs::{Dir, Metadata as CapMetadata, OpenOptions as CapOpenOptions};
use fs2::FileExt;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use tracing::debug;
use uuid::Uuid;

pub(super) fn remove_revalidated_entry(
    home: &MorphirHome,
    namespace: &str,
    relative: &str,
    trash_run: &TrashRun,
    index: usize,
) -> Result<(), CacheExecutionError> {
    let (parent, leaf, source) = open_removal_parent(home, namespace, relative)?;
    let destination_name = format!("{index:08x}");
    let destination = trash_run.path.join(&destination_name);
    parent
        .rename(&leaf, &trash_run.dir, &destination_name)
        .map_err(|error| io_error(&source, error))?;
    let metadata = trash_run
        .dir
        .symlink_metadata(&destination_name)
        .map_err(|source| io_error(&destination, source))?;
    if metadata.is_dir() && !cap_is_link_like(&metadata) {
        trash_run
            .dir
            .remove_dir_all(&destination_name)
            .map_err(|source| io_error(&destination, source))
    } else {
        trash_run
            .dir
            .remove_file(&destination_name)
            .map_err(|source| io_error(&destination, source))
    }
}

fn open_removal_parent(
    home: &MorphirHome,
    namespace: &str,
    relative: &str,
) -> Result<(Dir, String, PathBuf), CacheExecutionError> {
    let home_dir = Dir::open_ambient_dir(home.root(), ambient_authority())
        .map_err(|source| io_error(home.root(), source))?;
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
) -> Result<MaintenanceTrash, CacheExecutionError> {
    fs::create_dir_all(home.root()).map_err(|source| io_error(home.root(), source))?;
    let home_dir = Dir::open_ambient_dir(home.root(), ambient_authority())
        .map_err(|source| io_error(home.root(), source))?;
    let temp_path = home.temp_dir();
    let temp_dir = open_or_create_directory(&home_dir, "tmp", &temp_path)?;
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

pub(super) fn sweep_existing_trash(trash: &MaintenanceTrash) -> Result<(), CacheExecutionError> {
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
        remove_directory_contents(&run, &path)?;
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
    Ok(())
}

fn remove_directory_contents(
    directory: &Dir,
    display_path: &Path,
) -> Result<(), CacheExecutionError> {
    let mut entries = directory
        .entries()
        .map_err(|source| io_error(display_path, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| io_error(display_path, source))?;
    entries.sort_by_key(cap_std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        let path = display_path.join(&name);
        let metadata = directory
            .symlink_metadata(&name)
            .map_err(|source| io_error(&path, source))?;
        if metadata.is_dir() && !cap_is_link_like(&metadata) {
            directory
                .remove_dir_all(&name)
                .map_err(|source| io_error(&path, source))?;
        } else {
            directory
                .remove_file(&name)
                .map_err(|source| io_error(&path, source))?;
        }
    }
    Ok(())
}

fn is_trash_run_name(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
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

pub(super) struct MaintenanceGuard {
    file: File,
}

impl MaintenanceGuard {
    pub(super) fn acquire(home: &MorphirHome) -> Result<Self, CacheExecutionError> {
        fs::create_dir_all(home.root()).map_err(|source| io_error(home.root(), source))?;
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
            .map_err(|source| io_error(&path, source))?
            .into_std();
        FileExt::lock_exclusive(&file).map_err(|source| io_error(&path, source))?;
        Ok(Self { file })
    }
}

impl Drop for MaintenanceGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
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
mod tests {
    use super::{create_trash_run, open_maintenance_trash, remove_revalidated_entry};
    use crate::home::MorphirHome;
    use tempfile::TempDir;

    #[test]
    fn removal_refuses_a_link_like_source_ancestor() {
        let root = TempDir::new().unwrap();
        let home = MorphirHome::resolve_from(Some(root.path().as_os_str()), None).unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
        std::fs::write(outside.path().join("keep"), b"outside").unwrap();
        if create_directory_link(outside.path(), &home.downloads_cache_dir().join("owned")).is_err()
        {
            return;
        }
        let trash = open_maintenance_trash(&home).unwrap();
        let trash_run = create_trash_run(&trash).unwrap();

        assert!(remove_revalidated_entry(&home, "downloads", "owned/keep", &trash_run, 0).is_err());
        assert_eq!(
            std::fs::read(outside.path().join("keep")).unwrap(),
            b"outside"
        );
    }

    #[cfg(unix)]
    fn create_directory_link(
        target: &std::path::Path,
        link: &std::path::Path,
    ) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_directory_link(
        target: &std::path::Path,
        link: &std::path::Path,
    ) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }
}

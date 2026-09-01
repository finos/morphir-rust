use crate::home::MorphirHome;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
#[cfg(unix)]
use cap_fs_ext::{OpenOptionsMaybeDirExt, OpenOptionsSyncExt};
use cap_std::ambient_authority;
#[cfg(windows)]
use cap_std::fs::MetadataExt as CapMetadataExt;
use cap_std::fs::{Dir, Metadata as CapMetadata, OpenOptions as CapOpenOptions};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum DurableJsonError {
    #[error("durable JSON exceeds the {limit}-byte limit at {path}")]
    TooLarge { path: PathBuf, limit: u64 },
    #[error("refusing to use unsafe durable JSON path {path}")]
    UnsafePath { path: PathBuf },
    #[error("invalid durable JSON at {path}: {source}")]
    InvalidJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to encode durable JSON: {0}")]
    Encoding(#[source] serde_json::Error),
    #[error("durable JSON I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub(crate) struct DurableJsonSpec<'a> {
    pub(crate) path: &'a Path,
    pub(crate) filename: &'a str,
    pub(crate) staged_prefix: &'a str,
    pub(crate) max_bytes: u64,
}

pub(crate) fn load_ambient<T: DeserializeOwned + Default>(
    home: &MorphirHome,
    path: &Path,
    filename: &str,
    limit: u64,
) -> Result<T, DurableJsonError> {
    let root = match Dir::open_ambient_dir(home.root(), ambient_authority()) {
        Ok(root) => root,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(source) => return Err(io_error(home.root(), source)),
    };
    load_from_home(home, &root, path, filename, limit)
}

pub(crate) fn load_from_home<T: DeserializeOwned + Default>(
    home: &MorphirHome,
    root: &Dir,
    path: &Path,
    filename: &str,
    limit: u64,
) -> Result<T, DurableJsonError> {
    load_from_home_with_hook(home, root, path, filename, limit, || {})
}

pub(crate) fn load_from_home_with_hook<T, F>(
    home: &MorphirHome,
    root: &Dir,
    path: &Path,
    filename: &str,
    limit: u64,
    after_metadata: F,
) -> Result<T, DurableJsonError>
where
    T: DeserializeOwned + Default,
    F: FnOnce(),
{
    let Some(maintenance) = open_maintenance_directory(home, root)? else {
        return Ok(T::default());
    };
    let metadata = match maintenance.symlink_metadata(filename) {
        Ok(metadata) if cap_is_link_like(&metadata) || !metadata.is_file() => {
            return Err(DurableJsonError::UnsafePath {
                path: path.to_path_buf(),
            });
        }
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(source) => return Err(io_error(path, source)),
    };
    after_metadata();
    let mut options = CapOpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)]
    options.nonblock(true);
    let file = maintenance
        .open_with(filename, &options)
        .map_err(|source| io_error(path, source))?;
    let opened = file.metadata().map_err(|source| io_error(path, source))?;
    if cap_is_link_like(&opened) || !opened.is_file() {
        return Err(DurableJsonError::UnsafePath {
            path: path.to_path_buf(),
        });
    }
    if metadata.len() > limit || opened.len() > limit {
        return Err(DurableJsonError::TooLarge {
            path: path.to_path_buf(),
            limit,
        });
    }
    let mut bytes = Vec::with_capacity((limit + 1) as usize);
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    if bytes.len() as u64 > limit {
        return Err(DurableJsonError::TooLarge {
            path: path.to_path_buf(),
            limit,
        });
    }
    serde_json::from_slice(&bytes).map_err(|source| DurableJsonError::InvalidJson {
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn save_to_home<T: Serialize, F: FnOnce()>(
    home: &MorphirHome,
    root: &Dir,
    value: &T,
    spec: DurableJsonSpec<'_>,
    after_open: F,
) -> Result<(), DurableJsonError> {
    let DurableJsonSpec {
        path,
        filename,
        staged_prefix,
        max_bytes: limit,
    } = spec;
    let parent = path.parent().expect("durable JSON path has a parent");
    let maintenance = create_maintenance_directory(home, root, parent)?;
    validate_destination(&maintenance, path, filename)?;
    after_open();

    let mut bytes = serde_json::to_vec_pretty(value).map_err(DurableJsonError::Encoding)?;
    bytes.push(b'\n');
    if bytes.len() as u64 > limit {
        return Err(DurableJsonError::TooLarge {
            path: path.to_path_buf(),
            limit,
        });
    }
    let (staged_name, mut staged) = create_staged_file(&maintenance, parent, staged_prefix)?;
    let result = (|| {
        staged
            .write_all(&bytes)
            .and_then(|()| staged.flush())
            .and_then(|()| staged.sync_all())
            .map_err(|source| io_error(&parent.join(&staged_name), source))?;
        drop(staged);
        validate_destination(&maintenance, path, filename)?;
        install_staged_file(&maintenance, &staged_name, filename, parent, path)?;
        sync_directory(&maintenance, parent)
    })();
    if result.is_err() {
        let _ = maintenance.remove_file(&staged_name);
    }
    result
}

fn open_maintenance_directory(
    home: &MorphirHome,
    root: &Dir,
) -> Result<Option<Dir>, DurableJsonError> {
    let Some(data) = open_existing_directory(root, "data", &home.data_dir())? else {
        return Ok(None);
    };
    let maintenance_path = home.data_dir().join("maintenance");
    open_existing_directory(&data, "maintenance", &maintenance_path)
}

fn open_existing_directory(
    parent: &Dir,
    name: &str,
    path: &Path,
) -> Result<Option<Dir>, DurableJsonError> {
    match parent.symlink_metadata(name) {
        Ok(metadata) if cap_is_link_like(&metadata) || !metadata.is_dir() => {
            Err(DurableJsonError::UnsafePath {
                path: path.to_path_buf(),
            })
        }
        Ok(_) => parent
            .open_dir_nofollow(name)
            .map(Some)
            .map_err(|source| io_error(path, source)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(io_error(path, source)),
    }
}

fn create_maintenance_directory(
    home: &MorphirHome,
    root: &Dir,
    maintenance: &Path,
) -> Result<Dir, DurableJsonError> {
    let data_path = home.data_dir();
    let data = open_or_create_directory(root, "data", &data_path)?;
    sync_directory(root, home.root())?;
    let maintenance = open_or_create_directory(&data, "maintenance", maintenance)?;
    sync_directory(&data, &data_path)?;
    Ok(maintenance)
}

fn open_or_create_directory(
    parent: &Dir,
    name: &str,
    path: &Path,
) -> Result<Dir, DurableJsonError> {
    match parent.symlink_metadata(name) {
        Ok(metadata) if cap_is_link_like(&metadata) || !metadata.is_dir() => {
            Err(DurableJsonError::UnsafePath {
                path: path.to_path_buf(),
            })
        }
        Ok(_) => parent
            .open_dir_nofollow(name)
            .map_err(|source| io_error(path, source)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => match parent.create_dir(name)
        {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return open_or_create_directory(parent, name, path);
            }
            Err(source) => Err(io_error(path, source)),
        }
        .and_then(|()| {
            parent
                .open_dir_nofollow(name)
                .map_err(|source| io_error(path, source))
        }),
        Err(source) => Err(io_error(path, source)),
    }
}

fn validate_destination(
    maintenance: &Dir,
    path: &Path,
    filename: &str,
) -> Result<(), DurableJsonError> {
    match maintenance.symlink_metadata(filename) {
        Ok(metadata) if cap_is_link_like(&metadata) || !metadata.is_file() => {
            Err(DurableJsonError::UnsafePath {
                path: path.to_path_buf(),
            })
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

fn create_staged_file(
    maintenance: &Dir,
    parent: &Path,
    staged_prefix: &str,
) -> Result<(String, cap_std::fs::File), DurableJsonError> {
    for _ in 0..8 {
        let name = format!(".{staged_prefix}-{}", uuid::Uuid::new_v4().simple());
        let mut options = CapOpenOptions::new();
        options
            .create_new(true)
            .read(true)
            .write(true)
            .follow(FollowSymlinks::No);
        match maintenance.open_with(&name, &options) {
            Ok(file) => return Ok((name, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(io_error(&parent.join(name), source)),
        }
    }
    Err(io_error(
        parent,
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate a unique durable JSON staging file",
        ),
    ))
}

#[cfg(not(windows))]
fn install_staged_file(
    maintenance: &Dir,
    staged_name: &str,
    filename: &str,
    _parent: &Path,
    path: &Path,
) -> Result<(), DurableJsonError> {
    maintenance
        .rename(staged_name, maintenance, filename)
        .map_err(|source| io_error(path, source))
}

#[cfg(windows)]
fn install_staged_file(
    maintenance: &Dir,
    staged_name: &str,
    _filename: &str,
    parent: &Path,
    path: &Path,
) -> Result<(), DurableJsonError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let staged_metadata = maintenance
        .symlink_metadata(staged_name)
        .map_err(|source| io_error(&parent.join(staged_name), source))?;
    if cap_is_link_like(&staged_metadata) || !staged_metadata.is_file() {
        return Err(DurableJsonError::UnsafePath {
            path: parent.join(staged_name),
        });
    }
    let staged = parent.join(staged_name);
    let staged_wide = staged
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination_wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: Both UTF-16 buffers are NUL-terminated and alive for the call.
    let moved = unsafe {
        MoveFileExW(
            staged_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(io_error(path, std::io::Error::last_os_error()))
    } else {
        Ok(())
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

#[cfg(unix)]
fn sync_directory(directory: &Dir, path: &Path) -> Result<(), DurableJsonError> {
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
fn sync_directory(_directory: &Dir, _path: &Path) -> Result<(), DurableJsonError> {
    Ok(())
}

fn io_error(path: &Path, source: std::io::Error) -> DurableJsonError {
    DurableJsonError::Io {
        path: path.to_path_buf(),
        source,
    }
}

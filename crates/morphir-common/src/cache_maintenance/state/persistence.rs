use super::{CacheMaintenanceState, CacheMaintenanceStateError};
use crate::home::MorphirHome;
#[cfg(unix)]
use cap_fs_ext::OpenOptionsMaybeDirExt;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
#[cfg(windows)]
use cap_std::fs::MetadataExt as CapMetadataExt;
use cap_std::fs::{Dir, Metadata as CapMetadata, OpenOptions as CapOpenOptions};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use tracing::debug;

const MAX_CACHE_MAINTENANCE_STATE_BYTES: u64 = 64 * 1024;

/// Load bounded automatic-maintenance state, returning an empty state if absent.
///
/// Malformed, oversized, or link-like state fails closed so callers do not
/// accidentally reset the automatic-maintenance schedule.
///
/// ```
/// use morphir_common::cache_maintenance::{
///     CacheMaintenanceState, load_cache_maintenance_state,
/// };
/// use morphir_common::home::MorphirHome;
///
/// let temporary_home = tempfile::tempdir()?;
/// let home = MorphirHome::resolve_from(
///     Some(temporary_home.path().as_os_str()),
///     None,
/// )?;
/// let state = load_cache_maintenance_state(&home)?;
/// assert_eq!(state, CacheMaintenanceState::default());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn load_cache_maintenance_state(
    home: &MorphirHome,
) -> Result<CacheMaintenanceState, CacheMaintenanceStateError> {
    let path = home.cache_maintenance_state_file();
    let Some(maintenance) = open_state_directory(home)? else {
        return Ok(CacheMaintenanceState::default());
    };
    let metadata = match maintenance.symlink_metadata("cache-cleanup.json") {
        Ok(metadata) if cap_is_link_like(&metadata) || !metadata.is_file() => {
            return Err(CacheMaintenanceStateError::UnsafePath { path });
        }
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CacheMaintenanceState::default());
        }
        Err(source) => return Err(io_error(&path, source)),
    };
    let mut options = CapOpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = maintenance
        .open_with("cache-cleanup.json", &options)
        .map_err(|source| io_error(&path, source))?;
    let opened = file.metadata().map_err(|source| io_error(&path, source))?;
    if cap_is_link_like(&opened) || !opened.is_file() {
        return Err(CacheMaintenanceStateError::UnsafePath { path });
    }
    if metadata.len() > MAX_CACHE_MAINTENANCE_STATE_BYTES
        || opened.len() > MAX_CACHE_MAINTENANCE_STATE_BYTES
    {
        return Err(CacheMaintenanceStateError::StateTooLarge {
            path,
            limit: MAX_CACHE_MAINTENANCE_STATE_BYTES,
        });
    }
    let mut bytes = Vec::with_capacity((MAX_CACHE_MAINTENANCE_STATE_BYTES + 1) as usize);
    file.take(MAX_CACHE_MAINTENANCE_STATE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(&path, source))?;
    if bytes.len() as u64 > MAX_CACHE_MAINTENANCE_STATE_BYTES {
        return Err(CacheMaintenanceStateError::StateTooLarge {
            path,
            limit: MAX_CACHE_MAINTENANCE_STATE_BYTES,
        });
    }
    let state = serde_json::from_slice(&bytes).map_err(|source| {
        CacheMaintenanceStateError::InvalidState {
            path: path.clone(),
            source,
        }
    })?;
    debug!(
        event = "cache_maintenance_state_loaded",
        has_continuation = CacheMaintenanceState::continuation(&state).is_some(),
        "cache maintenance state loaded"
    );
    Ok(state)
}

fn open_state_directory(home: &MorphirHome) -> Result<Option<Dir>, CacheMaintenanceStateError> {
    let root = match Dir::open_ambient_dir(home.root(), ambient_authority()) {
        Ok(root) => root,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(io_error(home.root(), source)),
    };
    let Some(data) = open_existing_directory(&root, "data", &home.data_dir())? else {
        return Ok(None);
    };
    let maintenance_path = home.data_dir().join("maintenance");
    open_existing_directory(&data, "maintenance", &maintenance_path)
}

fn open_existing_directory(
    parent: &Dir,
    name: &str,
    path: &Path,
) -> Result<Option<Dir>, CacheMaintenanceStateError> {
    match parent.symlink_metadata(name) {
        Ok(metadata) if cap_is_link_like(&metadata) || !metadata.is_dir() => {
            Err(CacheMaintenanceStateError::UnsafePath {
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

/// Atomically replace durable automatic-maintenance state beneath Morphir Home.
///
/// ```
/// use morphir_common::cache_maintenance::{
///     CacheMaintenanceState, load_cache_maintenance_state,
///     save_cache_maintenance_state,
/// };
/// use morphir_common::home::MorphirHome;
///
/// let temporary_home = tempfile::tempdir()?;
/// let home = MorphirHome::resolve_from(
///     Some(temporary_home.path().as_os_str()),
///     None,
/// )?;
/// let state = CacheMaintenanceState::default().completed(1_000);
/// save_cache_maintenance_state(&home, &state)?;
/// assert_eq!(load_cache_maintenance_state(&home)?, state);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn save_cache_maintenance_state(
    home: &MorphirHome,
    state: &CacheMaintenanceState,
) -> Result<(), CacheMaintenanceStateError> {
    let _guard = super::super::executor::MaintenanceGuard::acquire(home)
        .map_err(CacheMaintenanceStateError::Coordination)?;
    save_cache_maintenance_state_under_guard(home, state)
}

pub(super) fn save_cache_maintenance_state_under_guard(
    home: &MorphirHome,
    state: &CacheMaintenanceState,
) -> Result<(), CacheMaintenanceStateError> {
    save_cache_maintenance_state_with_hook(home, state, || {})
}

fn save_cache_maintenance_state_with_hook<F>(
    home: &MorphirHome,
    state: &CacheMaintenanceState,
    after_open: F,
) -> Result<(), CacheMaintenanceStateError>
where
    F: FnOnce(),
{
    let path = home.cache_maintenance_state_file();
    let parent = path
        .parent()
        .expect("cache maintenance state path has a parent");
    let maintenance = create_state_directory(home, parent)?;
    validate_state_destination(&maintenance, &path)?;
    after_open();

    let mut bytes =
        serde_json::to_vec_pretty(state).map_err(CacheMaintenanceStateError::StateEncoding)?;
    bytes.push(b'\n');
    let (staged_name, mut staged) = create_staged_state_file(&maintenance, parent)?;
    let result = (|| {
        staged
            .write_all(&bytes)
            .and_then(|()| staged.flush())
            .and_then(|()| staged.sync_all())
            .map_err(|source| io_error(&parent.join(&staged_name), source))?;
        drop(staged);
        validate_state_destination(&maintenance, &path)?;
        install_staged_state(&maintenance, &staged_name, parent, &path)?;
        sync_state_directory(&maintenance, parent)
    })();
    if result.is_err() {
        let _ = maintenance.remove_file(&staged_name);
    }
    result?;
    debug!(
        event = "cache_maintenance_state_saved",
        has_continuation = state.continuation().is_some(),
        "cache maintenance state saved"
    );
    Ok(())
}

fn create_state_directory(
    home: &MorphirHome,
    maintenance: &Path,
) -> Result<Dir, CacheMaintenanceStateError> {
    fs::create_dir_all(home.root()).map_err(|source| io_error(home.root(), source))?;
    let root = Dir::open_ambient_dir(home.root(), ambient_authority())
        .map_err(|source| io_error(home.root(), source))?;
    let data_path = home.data_dir();
    let data = open_or_create_state_directory(&root, "data", &data_path)?;
    sync_state_directory(&root, home.root())?;
    let maintenance = open_or_create_state_directory(&data, "maintenance", maintenance)?;
    sync_state_directory(&data, &data_path)?;
    Ok(maintenance)
}

fn open_or_create_state_directory(
    parent: &Dir,
    name: &str,
    path: &Path,
) -> Result<Dir, CacheMaintenanceStateError> {
    match parent.symlink_metadata(name) {
        Ok(metadata) if cap_is_link_like(&metadata) || !metadata.is_dir() => {
            Err(CacheMaintenanceStateError::UnsafePath {
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
                return open_or_create_state_directory(parent, name, path);
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

fn validate_state_destination(
    maintenance: &Dir,
    path: &Path,
) -> Result<(), CacheMaintenanceStateError> {
    match maintenance.symlink_metadata("cache-cleanup.json") {
        Ok(metadata) if cap_is_link_like(&metadata) || !metadata.is_file() => {
            Err(CacheMaintenanceStateError::UnsafePath {
                path: path.to_path_buf(),
            })
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

fn create_staged_state_file(
    maintenance: &Dir,
    parent: &Path,
) -> Result<(String, cap_std::fs::File), CacheMaintenanceStateError> {
    for _ in 0..8 {
        let name = format!(".cache-cleanup-{}", uuid::Uuid::new_v4().simple());
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
            "could not allocate a unique cache maintenance state file",
        ),
    ))
}

#[cfg(not(windows))]
fn install_staged_state(
    maintenance: &Dir,
    staged_name: &str,
    _parent: &Path,
    path: &Path,
) -> Result<(), CacheMaintenanceStateError> {
    maintenance
        .rename(staged_name, maintenance, "cache-cleanup.json")
        .map_err(|source| io_error(path, source))
}

#[cfg(windows)]
fn install_staged_state(
    maintenance: &Dir,
    staged_name: &str,
    parent: &Path,
    path: &Path,
) -> Result<(), CacheMaintenanceStateError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let staged_metadata = maintenance
        .symlink_metadata(staged_name)
        .map_err(|source| io_error(&parent.join(staged_name), source))?;
    if cap_is_link_like(&staged_metadata) || !staged_metadata.is_file() {
        return Err(CacheMaintenanceStateError::UnsafePath {
            path: parent.join(staged_name),
        });
    }

    // cap-std opens Windows directory handles without FILE_SHARE_DELETE, so
    // holding `maintenance` pins this pathname while MoveFileExW performs the
    // platform's atomic, write-through replacement.
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
    // SAFETY: Both UTF-16 buffers are NUL-terminated and remain alive for the call.
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
fn sync_state_directory(maintenance: &Dir, path: &Path) -> Result<(), CacheMaintenanceStateError> {
    let mut options = CapOpenOptions::new();
    options
        .read(true)
        .maybe_dir(true)
        .follow(FollowSymlinks::No);
    maintenance
        .open_with(".", &options)
        .map(cap_std::fs::File::into_std)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error(path, source))
}

#[cfg(not(unix))]
fn sync_state_directory(
    _maintenance: &Dir,
    _path: &Path,
) -> Result<(), CacheMaintenanceStateError> {
    Ok(())
}

fn io_error(path: &Path, source: std::io::Error) -> CacheMaintenanceStateError {
    CacheMaintenanceStateError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_stays_with_the_pinned_directory_when_its_path_is_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let home = MorphirHome::resolve_from(Some(directory.path().as_os_str()), None).unwrap();
        let maintenance = home.data_dir().join("maintenance");
        let pinned = home.data_dir().join("maintenance-pinned");
        let outside = directory.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let mut swapped = false;

        save_cache_maintenance_state_with_hook(
            &home,
            &CacheMaintenanceState::default().completed(42),
            || {
                swapped = replace_directory_path(&maintenance, &pinned, &outside);
            },
        )
        .unwrap();

        let expected_parent = if swapped { &pinned } else { &maintenance };
        assert!(expected_parent.join("cache-cleanup.json").is_file());
        assert!(!outside.join("cache-cleanup.json").exists());
    }

    #[cfg(unix)]
    fn replace_directory_path(maintenance: &Path, pinned: &Path, outside: &Path) -> bool {
        fs::rename(maintenance, pinned).unwrap();
        std::os::unix::fs::symlink(outside, maintenance).unwrap();
        true
    }

    #[cfg(windows)]
    fn replace_directory_path(maintenance: &Path, pinned: &Path, outside: &Path) -> bool {
        match fs::rename(maintenance, pinned) {
            Ok(()) => {
                std::os::windows::fs::symlink_dir(outside, maintenance).unwrap();
                true
            }
            Err(_) => false,
        }
    }
}

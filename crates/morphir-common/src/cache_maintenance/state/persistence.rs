use super::{CacheMaintenanceState, CacheMaintenanceStateError};
use crate::cache_maintenance::durable_json::{self, DurableJsonError, DurableJsonSpec};
use crate::home::MorphirHome;
use cap_std::fs::Dir;
use tracing::debug;

const FILENAME: &str = "cache-cleanup.json";
const STAGED_PREFIX: &str = "cache-cleanup";
const MAX_BYTES: u64 = 64 * 1024;

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
    let state = durable_json::load_ambient(home, &path, FILENAME, MAX_BYTES)
        .map_err(CacheMaintenanceStateError::from)?;
    log_loaded(&state);
    Ok(state)
}

pub(super) fn load_cache_maintenance_state_under_guard(
    home: &MorphirHome,
    guard: &super::super::executor::MaintenanceGuard,
) -> Result<CacheMaintenanceState, CacheMaintenanceStateError> {
    load_cache_maintenance_state_from_home(home, guard.home_dir())
}

fn load_cache_maintenance_state_from_home(
    home: &MorphirHome,
    root: &Dir,
) -> Result<CacheMaintenanceState, CacheMaintenanceStateError> {
    let path = home.cache_maintenance_state_file();
    let state = durable_json::load_from_home(home, root, &path, FILENAME, MAX_BYTES)
        .map_err(CacheMaintenanceStateError::from)?;
    log_loaded(&state);
    Ok(state)
}

#[cfg(all(test, unix))]
fn load_cache_maintenance_state_from_home_with_hook<F>(
    home: &MorphirHome,
    root: &Dir,
    after_metadata: F,
) -> Result<CacheMaintenanceState, CacheMaintenanceStateError>
where
    F: FnOnce(),
{
    let path = home.cache_maintenance_state_file();
    durable_json::load_from_home_with_hook(home, root, &path, FILENAME, MAX_BYTES, after_metadata)
        .map_err(CacheMaintenanceStateError::from)
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
    let guard = super::super::executor::MaintenanceGuard::acquire(home)
        .map_err(CacheMaintenanceStateError::Coordination)?;
    save_cache_maintenance_state_under_guard(home, state, &guard)
}

pub(super) fn save_cache_maintenance_state_under_guard(
    home: &MorphirHome,
    state: &CacheMaintenanceState,
    guard: &super::super::executor::MaintenanceGuard,
) -> Result<(), CacheMaintenanceStateError> {
    save_cache_maintenance_state_with_home_hook(home, guard.home_dir(), state, || {})
}

#[cfg(test)]
fn save_cache_maintenance_state_with_hook<F>(
    home: &MorphirHome,
    state: &CacheMaintenanceState,
    after_open: F,
) -> Result<(), CacheMaintenanceStateError>
where
    F: FnOnce(),
{
    let guard = super::super::executor::MaintenanceGuard::acquire(home)
        .map_err(CacheMaintenanceStateError::Coordination)?;
    save_cache_maintenance_state_with_home_hook(home, guard.home_dir(), state, after_open)
}

fn save_cache_maintenance_state_with_home_hook<F>(
    home: &MorphirHome,
    root: &Dir,
    state: &CacheMaintenanceState,
    after_open: F,
) -> Result<(), CacheMaintenanceStateError>
where
    F: FnOnce(),
{
    let path = home.cache_maintenance_state_file();
    durable_json::save_to_home(
        home,
        root,
        state,
        DurableJsonSpec {
            path: &path,
            filename: FILENAME,
            staged_prefix: STAGED_PREFIX,
            max_bytes: MAX_BYTES,
        },
        after_open,
    )
    .map_err(CacheMaintenanceStateError::from)?;
    debug!(
        event = "cache_maintenance_state_saved",
        has_continuation = state.continuation().is_some(),
        "cache maintenance state saved"
    );
    Ok(())
}

fn log_loaded(state: &CacheMaintenanceState) {
    debug!(
        event = "cache_maintenance_state_loaded",
        has_continuation = state.continuation().is_some(),
        "cache maintenance state loaded"
    );
}

impl From<DurableJsonError> for CacheMaintenanceStateError {
    fn from(error: DurableJsonError) -> Self {
        match error {
            DurableJsonError::TooLarge { path, limit } => Self::StateTooLarge { path, limit },
            DurableJsonError::UnsafePath { path } => Self::UnsafePath { path },
            DurableJsonError::InvalidJson { path, source } => Self::InvalidState { path, source },
            DurableJsonError::Encoding(source) => Self::StateEncoding(source),
            DurableJsonError::Io { path, source } => Self::Io { path, source },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use cap_std::ambient_authority;
    use std::fs;
    use std::path::Path;

    #[cfg(unix)]
    #[test]
    fn state_open_rejects_a_fifo_swapped_in_after_metadata_without_blocking() {
        use std::os::unix::ffi::OsStrExt;

        let directory = tempfile::tempdir().unwrap();
        let home = MorphirHome::resolve_from(Some(directory.path().as_os_str()), None).unwrap();
        save_cache_maintenance_state(&home, &CacheMaintenanceState::default()).unwrap();
        let path = home.cache_maintenance_state_file();
        let root = Dir::open_ambient_dir(home.root(), ambient_authority()).unwrap();

        let error = load_cache_maintenance_state_from_home_with_hook(&home, &root, || {
            fs::remove_file(&path).unwrap();
            let path_bytes = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
            // SAFETY: `path_bytes` is NUL-terminated and remains valid for the call.
            assert_eq!(unsafe { libc::mkfifo(path_bytes.as_ptr(), 0o600) }, 0);
        })
        .unwrap_err();

        assert!(matches!(
            error,
            CacheMaintenanceStateError::UnsafePath { .. }
        ));
    }

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
            || swapped = replace_directory_path(&maintenance, &pinned, &outside),
        )
        .unwrap();

        let expected_parent = if swapped { &pinned } else { &maintenance };
        assert!(expected_parent.join(FILENAME).is_file());
        assert!(!outside.join(FILENAME).exists());
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

use super::CacheModelError;
use crate::home::MorphirHome;
use serde::{Deserialize, Deserializer, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;
use tracing::debug;

const CACHE_MAINTENANCE_STATE_SCHEMA_VERSION: u32 = 1;
const MAX_CACHE_MAINTENANCE_STATE_BYTES: u64 = 64 * 1024;

/// A durable continuation position in deterministic namespace/path order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheCleanupCursor {
    namespace: String,
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CacheCleanupCursorWire {
    namespace: String,
    path: String,
}

impl<'de> Deserialize<'de> for CacheCleanupCursor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CacheCleanupCursorWire::deserialize(deserializer)?;
        Self::new(wire.namespace, wire.path).map_err(serde::de::Error::custom)
    }
}

impl CacheCleanupCursor {
    /// Construct a cursor from a registered namespace and portable entry path.
    pub fn new(
        namespace: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<Self, CacheModelError> {
        let namespace = namespace.into();
        let path = path.into();
        super::CacheEntry::unclassified(namespace.clone(), path.clone(), 0)?;
        Ok(Self { namespace, path })
    }

    /// Registered namespace of the next entry to consider.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Portable path of the next entry to consider.
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// Durable state shared by CLI and Desktop automatic cache maintenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheMaintenanceState {
    schema_version: u32,
    last_successful_automatic_run: Option<u64>,
    continuation: Option<CacheCleanupCursor>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CacheMaintenanceStateWire {
    schema_version: u32,
    #[serde(default)]
    last_successful_automatic_run: Option<u64>,
    #[serde(default)]
    continuation: Option<CacheCleanupCursor>,
}

impl<'de> Deserialize<'de> for CacheMaintenanceState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CacheMaintenanceStateWire::deserialize(deserializer)?;
        if wire.schema_version != CACHE_MAINTENANCE_STATE_SCHEMA_VERSION {
            return Err(serde::de::Error::custom(format!(
                "unsupported cache maintenance state schema version {}",
                wire.schema_version
            )));
        }
        Ok(Self {
            schema_version: wire.schema_version,
            last_successful_automatic_run: wire.last_successful_automatic_run,
            continuation: wire.continuation,
        })
    }
}

impl Default for CacheMaintenanceState {
    fn default() -> Self {
        Self {
            schema_version: CACHE_MAINTENANCE_STATE_SCHEMA_VERSION,
            last_successful_automatic_run: None,
            continuation: None,
        }
    }
}

impl CacheMaintenanceState {
    /// Timestamp of the last automatic pass that reached the end of inventory.
    pub fn last_successful_automatic_run(&self) -> Option<u64> {
        self.last_successful_automatic_run
    }

    /// Next deterministic inventory position after a bounded partial pass.
    pub fn continuation(&self) -> Option<&CacheCleanupCursor> {
        self.continuation.as_ref()
    }

    /// Record a complete automatic pass and clear any continuation.
    #[must_use]
    pub fn completed(mut self, completed_at: u64) -> Self {
        self.last_successful_automatic_run = Some(completed_at);
        self.continuation = None;
        self
    }

    /// Record a bounded partial pass without advancing the successful-run time.
    #[must_use]
    pub fn continued(mut self, cursor: CacheCleanupCursor) -> Self {
        self.continuation = Some(cursor);
        self
    }
}

/// Whether an automatic cleanup opportunity should run now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AutomaticCacheCleanupDecision {
    /// No completed pass exists, the interval elapsed, or continuation remains.
    Due,
    /// The most recent complete pass still falls within the configured interval.
    Deferred {
        /// Earliest Unix timestamp at which another complete pass is due.
        next_run: u64,
    },
}

/// Errors reading, writing, or evaluating automatic cache-maintenance state.
#[derive(Debug, Error)]
pub enum CacheMaintenanceStateError {
    /// Automatic maintenance intervals must be nonzero.
    #[error("automatic cache cleanup interval must be nonzero")]
    InvalidInterval,
    /// The suite-wide maintenance lock could not be acquired.
    #[error("cache maintenance state coordination failed: {0}")]
    Coordination(#[source] super::CacheExecutionError),
    /// The state file exceeds its bounded input size.
    #[error("cache maintenance state exceeds the {limit}-byte limit at {path}")]
    StateTooLarge {
        /// State file being read.
        path: PathBuf,
        /// Maximum accepted bytes.
        limit: u64,
    },
    /// A state path is a link-like object or has an unexpected type.
    #[error("refusing to use unsafe cache maintenance state path {path}")]
    UnsafePath {
        /// Path that failed validation.
        path: PathBuf,
    },
    /// Persisted JSON is malformed or uses an unsupported schema.
    #[error("invalid cache maintenance state at {path}: {source}")]
    InvalidState {
        /// State file being decoded.
        path: PathBuf,
        /// JSON decoding or validation error.
        #[source]
        source: serde_json::Error,
    },
    /// State could not be encoded.
    #[error("failed to encode cache maintenance state: {0}")]
    StateEncoding(#[source] serde_json::Error),
    /// A filesystem operation failed.
    #[error("cache maintenance state I/O failed at {path}: {source}")]
    Io {
        /// Path involved in the failed operation.
        path: PathBuf,
        /// Underlying filesystem error.
        #[source]
        source: std::io::Error,
    },
}

/// Evaluate interval gating without changing durable state.
///
/// ```
/// use morphir_common::cache_maintenance::{
///     AutomaticCacheCleanupDecision, CacheMaintenanceState,
///     automatic_cache_cleanup_decision,
/// };
/// use std::time::Duration;
///
/// let state = CacheMaintenanceState::default().completed(1_000);
/// let decision = automatic_cache_cleanup_decision(
///     &state,
///     1_100,
///     Duration::from_secs(200),
/// )?;
/// assert_eq!(
///     decision,
///     AutomaticCacheCleanupDecision::Deferred { next_run: 1_200 }
/// );
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn automatic_cache_cleanup_decision(
    state: &CacheMaintenanceState,
    now: u64,
    interval: Duration,
) -> Result<AutomaticCacheCleanupDecision, CacheMaintenanceStateError> {
    if interval.as_secs() == 0 {
        return Err(CacheMaintenanceStateError::InvalidInterval);
    }
    if state.continuation.is_some() {
        return Ok(AutomaticCacheCleanupDecision::Due);
    }
    let Some(last_run) = state.last_successful_automatic_run else {
        return Ok(AutomaticCacheCleanupDecision::Due);
    };
    let next_run = last_run.saturating_add(interval.as_secs());
    if now >= next_run {
        Ok(AutomaticCacheCleanupDecision::Due)
    } else {
        Ok(AutomaticCacheCleanupDecision::Deferred { next_run })
    }
}

/// Load bounded automatic-maintenance state, returning an empty state if absent.
pub fn load_cache_maintenance_state(
    home: &MorphirHome,
) -> Result<CacheMaintenanceState, CacheMaintenanceStateError> {
    let path = home.cache_maintenance_state_file();
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CacheMaintenanceState::default());
        }
        Err(source) => return Err(io_error(&path, source)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CacheMaintenanceStateError::UnsafePath { path });
    }
    if metadata.len() > MAX_CACHE_MAINTENANCE_STATE_BYTES {
        return Err(CacheMaintenanceStateError::StateTooLarge {
            path,
            limit: MAX_CACHE_MAINTENANCE_STATE_BYTES,
        });
    }
    let file = fs::File::open(&path).map_err(|source| io_error(&path, source))?;
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

/// Atomically replace durable automatic-maintenance state beneath Morphir Home.
pub fn save_cache_maintenance_state(
    home: &MorphirHome,
    state: &CacheMaintenanceState,
) -> Result<(), CacheMaintenanceStateError> {
    let _guard = super::executor::MaintenanceGuard::acquire(home)
        .map_err(CacheMaintenanceStateError::Coordination)?;
    let path = home.cache_maintenance_state_file();
    let parent = path
        .parent()
        .expect("cache maintenance state path has a parent");
    create_state_directory(home, parent)?;
    validate_state_destination(&path)?;

    let mut bytes =
        serde_json::to_vec_pretty(state).map_err(CacheMaintenanceStateError::StateEncoding)?;
    bytes.push(b'\n');
    let mut staged = tempfile::Builder::new()
        .prefix(".cache-cleanup-")
        .tempfile_in(parent)
        .map_err(|source| io_error(parent, source))?;
    staged
        .as_file_mut()
        .write_all(&bytes)
        .and_then(|()| staged.as_file_mut().flush())
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|source| io_error(staged.path(), source))?;
    validate_state_destination(&path)?;
    staged
        .persist(&path)
        .map_err(|error| io_error(&path, error.error))?;
    sync_parent_directory(&path)?;
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
) -> Result<(), CacheMaintenanceStateError> {
    fs::create_dir_all(home.root()).map_err(|source| io_error(home.root(), source))?;
    let data = home.data_dir();
    create_checked_directory(&data)?;
    create_checked_directory(maintenance)
}

fn create_checked_directory(path: &Path) -> Result<(), CacheMaintenanceStateError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(CacheMaintenanceStateError::UnsafePath {
                path: path.to_path_buf(),
            })
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => match fs::create_dir(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                create_checked_directory(path)
            }
            Err(source) => Err(io_error(path, source)),
        },
        Err(source) => Err(io_error(path, source)),
    }
}

fn validate_state_destination(path: &Path) -> Result<(), CacheMaintenanceStateError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(CacheMaintenanceStateError::UnsafePath {
                path: path.to_path_buf(),
            })
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<(), CacheMaintenanceStateError> {
    let parent = path
        .parent()
        .expect("cache maintenance state path has a parent");
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error(parent, source))
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> Result<(), CacheMaintenanceStateError> {
    Ok(())
}

fn io_error(path: &Path, source: std::io::Error) -> CacheMaintenanceStateError {
    CacheMaintenanceStateError::Io {
        path: path.to_path_buf(),
        source,
    }
}

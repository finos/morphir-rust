use super::{CacheOwnershipRegistry, CacheOwnershipRegistryError};
use crate::cache_maintenance::durable_json::{self, DurableJsonError, DurableJsonSpec};
use crate::cache_maintenance::executor::MaintenanceGuard;
use crate::home::MorphirHome;
use std::path::PathBuf;
use thiserror::Error;
use tracing::debug;

const FILENAME: &str = "cache-ownership.json";
const STAGED_PREFIX: &str = "cache-ownership";
const MAX_BYTES: u64 = 64 * 1024;

/// Errors coordinating or persisting trusted cache ownership declarations.
#[derive(Debug, Error)]
pub enum CacheOwnershipPersistenceError {
    /// Suite-wide maintenance coordination failed.
    #[error("cache ownership coordination failed: {0}")]
    Coordination(#[source] super::super::CacheExecutionError),
    /// A producer supplied invalid ownership metadata.
    #[error(transparent)]
    Registry(#[from] CacheOwnershipRegistryError),
    /// The bounded registry input or output is too large.
    #[error("cache ownership registry exceeds the {limit}-byte limit at {path}")]
    RegistryTooLarge { path: PathBuf, limit: u64 },
    /// A registry path is link-like or has an unexpected type.
    #[error("refusing to use unsafe cache ownership registry path {path}")]
    UnsafePath { path: PathBuf },
    /// Persisted JSON is malformed or uses an unsupported schema.
    #[error("invalid cache ownership registry at {path}: {source}")]
    InvalidRegistry {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    /// Registry JSON could not be encoded.
    #[error("failed to encode cache ownership registry: {0}")]
    RegistryEncoding(#[source] serde_json::Error),
    /// A filesystem operation failed.
    #[error("cache ownership registry I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Load the trusted cache ownership registry under suite-wide coordination.
pub fn load_cache_ownership_registry(
    home: &MorphirHome,
) -> Result<CacheOwnershipRegistry, CacheOwnershipPersistenceError> {
    let guard =
        MaintenanceGuard::acquire(home).map_err(CacheOwnershipPersistenceError::Coordination)?;
    load_cache_ownership_registry_under_guard(home, &guard)
}

/// Register or refresh one disposable entry after its cache mutation completes.
pub fn register_cache_ownership(
    home: &MorphirHome,
    namespace: impl Into<String>,
    path: impl Into<String>,
    last_used: u64,
) -> Result<(), CacheOwnershipPersistenceError> {
    let guard =
        MaintenanceGuard::acquire(home).map_err(CacheOwnershipPersistenceError::Coordination)?;
    let mut registry = load_cache_ownership_registry_under_guard(home, &guard)?;
    registry.register_disposable(namespace, path, last_used)?;
    save_cache_ownership_registry_under_guard(home, &registry, &guard)
}

/// Stop declaring ownership without deleting the cache content itself.
pub fn unregister_cache_ownership(
    home: &MorphirHome,
    namespace: &str,
    path: &str,
) -> Result<bool, CacheOwnershipPersistenceError> {
    let guard =
        MaintenanceGuard::acquire(home).map_err(CacheOwnershipPersistenceError::Coordination)?;
    let mut registry = load_cache_ownership_registry_under_guard(home, &guard)?;
    let removed = registry.unregister(namespace, path)?;
    if removed {
        save_cache_ownership_registry_under_guard(home, &registry, &guard)?;
    }
    Ok(removed)
}

pub(crate) fn load_cache_ownership_registry_under_guard(
    home: &MorphirHome,
    guard: &MaintenanceGuard,
) -> Result<CacheOwnershipRegistry, CacheOwnershipPersistenceError> {
    let path = home.cache_ownership_registry_file();
    let registry = durable_json::load_from_home(home, guard.home_dir(), &path, FILENAME, MAX_BYTES)
        .map_err(CacheOwnershipPersistenceError::from)?;
    debug!(
        event = "cache_ownership_registry_loaded",
        entry_count = CacheOwnershipRegistry::len(&registry),
        "cache ownership registry loaded"
    );
    Ok(registry)
}

fn save_cache_ownership_registry_under_guard(
    home: &MorphirHome,
    registry: &CacheOwnershipRegistry,
    guard: &MaintenanceGuard,
) -> Result<(), CacheOwnershipPersistenceError> {
    let path = home.cache_ownership_registry_file();
    durable_json::save_to_home(
        home,
        guard.home_dir(),
        registry,
        DurableJsonSpec {
            path: &path,
            filename: FILENAME,
            staged_prefix: STAGED_PREFIX,
            max_bytes: MAX_BYTES,
        },
        || {},
    )
    .map_err(CacheOwnershipPersistenceError::from)?;
    debug!(
        event = "cache_ownership_registry_saved",
        entry_count = registry.len(),
        "cache ownership registry saved"
    );
    Ok(())
}

impl From<DurableJsonError> for CacheOwnershipPersistenceError {
    fn from(error: DurableJsonError) -> Self {
        match error {
            DurableJsonError::TooLarge { path, limit } => Self::RegistryTooLarge { path, limit },
            DurableJsonError::UnsafePath { path } => Self::UnsafePath { path },
            DurableJsonError::InvalidJson { path, source } => {
                Self::InvalidRegistry { path, source }
            }
            DurableJsonError::Encoding(source) => Self::RegistryEncoding(source),
            DurableJsonError::Io { path, source } => Self::Io { path, source },
        }
    }
}

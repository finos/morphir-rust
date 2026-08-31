use super::{CacheOwnershipRegistry, CacheOwnershipRegistryError};
use crate::cache_maintenance::durable_json::{self, DurableJsonError, DurableJsonSpec};
use crate::cache_maintenance::executor::{
    CacheMutationGuard, CacheOwnershipWriteGuard, MaintenanceGuard,
};
use crate::home::MorphirHome;
use cap_std::fs::Dir;
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

pub(crate) fn load_cache_ownership_registry_under_guard(
    home: &MorphirHome,
    guard: &MaintenanceGuard,
) -> Result<CacheOwnershipRegistry, CacheOwnershipPersistenceError> {
    load_cache_ownership_registry_from_home(home, guard.home_dir())
}

fn load_cache_ownership_registry_from_home(
    home: &MorphirHome,
    home_dir: &Dir,
) -> Result<CacheOwnershipRegistry, CacheOwnershipPersistenceError> {
    let path = home.cache_ownership_registry_file();
    let registry = durable_json::load_from_home(home, home_dir, &path, FILENAME, MAX_BYTES)
        .map_err(CacheOwnershipPersistenceError::from)?;
    debug!(
        event = "cache_ownership_registry_loaded",
        entry_count = CacheOwnershipRegistry::len(&registry),
        "cache ownership registry loaded"
    );
    Ok(registry)
}

fn save_cache_ownership_registry_to_home(
    home: &MorphirHome,
    registry: &CacheOwnershipRegistry,
    home_dir: &Dir,
) -> Result<(), CacheOwnershipPersistenceError> {
    let path = home.cache_ownership_registry_file();
    durable_json::save_to_home(
        home,
        home_dir,
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

impl CacheMutationGuard {
    /// Finish a cache mutation by durably registering or refreshing ownership.
    ///
    /// The shared mutation lease remains held until the registry replacement is
    /// durable, so cleanup cannot act on the previous `last_used` timestamp in
    /// the handoff between cache use and ownership publication.
    pub fn finish_with_ownership(
        self,
        namespace: impl Into<String>,
        path: impl Into<String>,
        last_used: u64,
    ) -> Result<(), CacheOwnershipPersistenceError> {
        let namespace = namespace.into();
        let _write_guard = CacheOwnershipWriteGuard::acquire(self.home())
            .map_err(CacheOwnershipPersistenceError::Coordination)?;
        let mut registry = load_cache_ownership_registry_from_home(self.home(), self.home_dir())?;
        registry.register_disposable(&namespace, path, last_used)?;
        save_cache_ownership_registry_to_home(self.home(), &registry, self.home_dir())?;
        debug!(
            event = "cache_ownership_registered",
            namespace,
            entry_count = registry.len(),
            "cache ownership registered"
        );
        Ok(())
    }

    /// Finish a cache mutation by durably releasing ownership of one entry.
    ///
    /// Cleanup remains excluded until the entry is unclassified in the durable
    /// registry. The cache content itself is not modified.
    pub fn finish_releasing_ownership(
        self,
        namespace: &str,
        path: &str,
    ) -> Result<bool, CacheOwnershipPersistenceError> {
        let _write_guard = CacheOwnershipWriteGuard::acquire(self.home())
            .map_err(CacheOwnershipPersistenceError::Coordination)?;
        let mut registry = load_cache_ownership_registry_from_home(self.home(), self.home_dir())?;
        let removed = registry.unregister(namespace, path)?;
        if removed {
            save_cache_ownership_registry_to_home(self.home(), &registry, self.home_dir())?;
        }
        debug!(
            event = "cache_ownership_unregistered",
            namespace,
            removed,
            entry_count = registry.len(),
            "cache ownership unregistered"
        );
        Ok(removed)
    }
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

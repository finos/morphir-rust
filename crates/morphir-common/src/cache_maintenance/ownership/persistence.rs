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

/// Ownership-aware mutation lease for one registered cache identity.
///
/// Beginning the mutation durably removes any prior registration that overlaps
/// the mutation path before this capability is returned. The suite-wide
/// mutation lease and ownership writer lock remain held until the capability is
/// dropped or finished, so another producer cannot publish overlapping
/// ownership while the path may still be changing.
///
/// ```no_run
/// use morphir_common::cache_maintenance::CacheOwnershipMutationGuard;
/// use morphir_common::home::MorphirHome;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let home = MorphirHome::resolve()?;
/// let mutation = CacheOwnershipMutationGuard::begin(
///     &home,
///     "downloads",
///     "desktop/1.2.3.pkg",
/// )?;
/// // Write and close cache/downloads/desktop/1.2.3.pkg here.
/// mutation.finish(1_735_689_600)?;
/// # Ok(())
/// # }
/// ```
pub struct CacheOwnershipMutationGuard {
    guard: CacheMutationGuard,
    _ownership_guard: CacheOwnershipWriteGuard,
    namespace: String,
    path: String,
    invalidated_last_used: Option<u64>,
}

impl CacheOwnershipMutationGuard {
    /// Begin mutating one cache identity after durably invalidating overlapping ownership.
    ///
    /// If the producer exits before [`Self::finish`], the entry remains absent
    /// from the trusted registry and cleanup therefore treats its content as
    /// unclassified.
    pub fn begin(
        home: &MorphirHome,
        namespace: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<Self, CacheOwnershipPersistenceError> {
        Self::begin_with_hook(home, namespace, path, || {})
    }

    fn begin_with_hook<F>(
        home: &MorphirHome,
        namespace: impl Into<String>,
        path: impl Into<String>,
        after_mutation_lock: F,
    ) -> Result<Self, CacheOwnershipPersistenceError>
    where
        F: FnOnce(),
    {
        let namespace = namespace.into();
        let path = path.into();
        let guard = CacheMutationGuard::acquire(home)
            .map_err(CacheOwnershipPersistenceError::Coordination)?;
        after_mutation_lock();
        let ownership_guard = CacheOwnershipWriteGuard::acquire(home, guard.home_dir())
            .map_err(CacheOwnershipPersistenceError::Coordination)?;
        let mut registry = load_cache_ownership_registry_from_home(home, guard.home_dir())?;
        let (invalidated_entries, invalidated_last_used) =
            registry.unregister_overlapping_with_last_used(&namespace, &path)?;
        if invalidated_entries > 0 {
            save_cache_ownership_registry_to_home(home, &registry, guard.home_dir())?;
        }
        debug!(
            event = "cache_ownership_mutation_begun",
            namespace,
            invalidated_entries,
            entry_count = registry.len(),
            "cache ownership invalidated before mutation"
        );
        Ok(Self {
            guard,
            _ownership_guard: ownership_guard,
            namespace,
            path,
            invalidated_last_used,
        })
    }

    /// Namespace whose ownership was invalidated before mutation began.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Portable path whose ownership was invalidated before mutation began.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Finish the mutation by durably registering the same cache identity.
    ///
    /// A failed handoff returns [`CacheOwnershipHandoffError`], which retains
    /// both coordination locks so cleanup remains excluded during recovery.
    pub fn finish(self, last_used: u64) -> Result<(), CacheOwnershipHandoffError> {
        let result = self.publish_ownership(last_used);
        result.map_err(|source| CacheOwnershipHandoffError {
            guard: Box::new(self),
            source,
        })
    }

    fn publish_ownership(&self, last_used: u64) -> Result<(), CacheOwnershipPersistenceError> {
        let last_used = self
            .invalidated_last_used
            .map_or(last_used, |previous| previous.max(last_used));
        let mut registry =
            load_cache_ownership_registry_from_home(self.guard.home(), self.guard.home_dir())?;
        registry.register_disposable(&self.namespace, &self.path, last_used)?;
        save_cache_ownership_registry_to_home(self.guard.home(), &registry, self.guard.home_dir())?;
        debug!(
            event = "cache_ownership_registered",
            namespace = self.namespace,
            entry_count = registry.len(),
            "cache ownership registered"
        );
        Ok(())
    }

    /// Finish without republishing ownership, leaving the content protected.
    ///
    /// Returns whether overlapping ownership was registered when the mutation began.
    pub fn finish_unowned(self) -> bool {
        self.invalidated_last_used.is_some()
    }
}

/// Failed producer-to-maintenance handoff that retains the mutation lease.
///
/// Call [`Self::into_parts`] to recover the guard and keep cleanup excluded
/// while retrying, quarantining, or otherwise handling the cache content.
pub struct CacheOwnershipHandoffError {
    guard: Box<CacheOwnershipMutationGuard>,
    source: CacheOwnershipPersistenceError,
}

impl CacheOwnershipHandoffError {
    /// Recover the still-held mutation lease and the publication failure.
    pub fn into_parts(self) -> (CacheOwnershipMutationGuard, CacheOwnershipPersistenceError) {
        (*self.guard, self.source)
    }
}

impl std::fmt::Debug for CacheOwnershipHandoffError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CacheOwnershipHandoffError")
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for CacheOwnershipHandoffError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(formatter)
    }
}

impl std::error::Error for CacheOwnershipHandoffError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Load the trusted cache ownership registry under suite-wide coordination.
///
/// ```
/// use morphir_common::cache_maintenance::load_cache_ownership_registry;
/// use morphir_common::home::MorphirHome;
///
/// let temporary_home = tempfile::tempdir()?;
/// let home = MorphirHome::resolve_from(
///     Some(temporary_home.path().as_os_str()),
///     None,
/// )?;
/// let registry = load_cache_ownership_registry(&home)?;
/// assert!(registry.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
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

pub(crate) fn save_cache_ownership_registry_under_guard(
    home: &MorphirHome,
    registry: &CacheOwnershipRegistry,
    guard: &MaintenanceGuard,
) -> Result<(), CacheOwnershipPersistenceError> {
    save_cache_ownership_registry_to_home(home, registry, guard.home_dir())
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn ownership_mutation_stays_with_the_pinned_home_when_its_path_is_replaced() {
        let container = tempfile::tempdir().unwrap();
        let active = container.path().join("active");
        let pinned = container.path().join("pinned");
        std::fs::create_dir(&active).unwrap();
        let home = MorphirHome::resolve_from(Some(active.as_os_str()), None).unwrap();

        let mutation = CacheOwnershipMutationGuard::begin_with_hook(
            &home,
            "downloads",
            "artifact.pkg",
            || {
                std::fs::rename(&active, &pinned).unwrap();
                std::fs::create_dir(&active).unwrap();
            },
        )
        .unwrap();

        assert!(pinned.join("locks/cache-ownership.lock").is_file());
        assert!(!active.join("locks/cache-ownership.lock").exists());
        mutation.finish(10).unwrap();

        let pinned_home = MorphirHome::resolve_from(Some(pinned.as_os_str()), None).unwrap();
        let replacement_home = MorphirHome::resolve_from(Some(active.as_os_str()), None).unwrap();
        assert_eq!(
            load_cache_ownership_registry(&pinned_home).unwrap().len(),
            1
        );
        assert!(
            load_cache_ownership_registry(&replacement_home)
                .unwrap()
                .is_empty()
        );
    }
}

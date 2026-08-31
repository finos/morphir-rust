use super::executor::MaintenanceGuard;
use super::ownership::load_cache_ownership_registry_under_guard;
use super::{
    CacheEntry, CacheExecutionError, CacheExecutionLimits, CacheExecutionReport,
    CacheInventoryError, CacheInventoryLimits, CacheNamespace, CacheOwnershipPersistenceError,
    CacheOwnershipRegistry, CleanupPlan,
};
use crate::home::MorphirHome;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Errors beginning or using an exclusive cache-maintenance session.
#[derive(Debug, Error)]
pub enum CacheMaintenanceSessionError {
    /// Suite-wide coordination could not be acquired.
    #[error("cache maintenance coordination failed: {0}")]
    Coordination(#[source] CacheExecutionError),
    /// Trusted ownership metadata could not be loaded.
    #[error(transparent)]
    Ownership(#[from] CacheOwnershipPersistenceError),
    /// A registered namespace could not be inventoried safely.
    #[error(transparent)]
    Inventory(#[from] CacheInventoryError),
    /// Trusted ownership metadata could not be converted to namespaces.
    #[error(transparent)]
    Registration(#[from] super::CacheOwnershipRegistryError),
    /// A namespace was requested more than once.
    #[error("duplicate requested cache namespace {namespace}")]
    DuplicateNamespace {
        /// Repeated namespace identifier.
        namespace: String,
    },
    /// A cleanup plan could not be executed safely.
    #[error(transparent)]
    Execution(#[from] CacheExecutionError),
}

/// Exclusive cache-maintenance capability pinned to one Morphir Home.
///
/// The ownership snapshot is loaded only after the suite-wide lock is held.
/// Keeping the session alive across inventory, planning, and execution prevents
/// cleanup from acting on ownership metadata that a producer refreshed or
/// released concurrently.
///
/// ```no_run
/// use morphir_common::cache_maintenance::{
///     CacheExecutionLimits, CacheInventoryLimits, CacheMaintenanceSession,
///     CachePolicy, CleanupMode, plan_cache_cleanup,
/// };
/// use morphir_common::home::MorphirHome;
/// use std::time::Duration;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let home = MorphirHome::resolve()?;
/// let session = CacheMaintenanceSession::begin(&home)?;
/// let inventory = session.inventory(
///     &["desktop", "downloads", "extensions", "indexes"],
///     CacheInventoryLimits::default(),
/// )?;
/// let plan = plan_cache_cleanup(
///     inventory,
///     CachePolicy::new(Duration::from_secs(30 * 24 * 60 * 60), 2_000_000_000),
///     1_735_689_600,
///     CleanupMode::Policy,
/// )?;
/// let report = session.execute_cleanup(
///     &plan,
///     CacheInventoryLimits::default(),
///     CacheExecutionLimits::new(100, 100_000_000)?,
/// )?;
/// println!("removed {} bytes", report.removed_bytes());
/// # Ok(())
/// # }
/// ```
pub struct CacheMaintenanceSession<'home> {
    home: &'home MorphirHome,
    ownership: CacheOwnershipRegistry,
    guard: MaintenanceGuard,
}

impl<'home> CacheMaintenanceSession<'home> {
    /// Acquire exclusive coordination and load the current trusted registry.
    pub fn begin(home: &'home MorphirHome) -> Result<Self, CacheMaintenanceSessionError> {
        let guard =
            MaintenanceGuard::acquire(home).map_err(CacheMaintenanceSessionError::Coordination)?;
        let ownership = load_cache_ownership_registry_under_guard(home, &guard)?;
        Ok(Self {
            home,
            ownership,
            guard,
        })
    }

    /// Trusted ownership snapshot loaded while this session held coordination.
    pub fn ownership(&self) -> &CacheOwnershipRegistry {
        &self.ownership
    }

    /// Inventory named namespaces through this session's pinned Morphir Home.
    ///
    /// Registered ownership is always taken from the trusted snapshot. A valid
    /// namespace with no registrations is still inventoried, but every observed
    /// entry remains protected and unclassified. An empty name list inventories
    /// every registered namespace.
    pub fn inventory(
        &self,
        namespace_names: &[&str],
        limits: CacheInventoryLimits,
    ) -> Result<Vec<CacheEntry>, CacheMaintenanceSessionError> {
        let registered = self
            .ownership
            .namespaces()?
            .into_iter()
            .map(|namespace| (namespace.name().to_owned(), namespace))
            .collect::<BTreeMap<_, _>>();
        let namespaces = if namespace_names.is_empty() {
            registered.into_values().collect::<Vec<_>>()
        } else {
            let mut seen = BTreeSet::new();
            let mut selected = Vec::with_capacity(namespace_names.len());
            for name in namespace_names {
                if !seen.insert(*name) {
                    return Err(CacheMaintenanceSessionError::DuplicateNamespace {
                        namespace: (*name).to_owned(),
                    });
                }
                selected.push(match registered.get(*name) {
                    Some(namespace) => namespace.clone(),
                    None => CacheNamespace::new(*name).map_err(|error| {
                        CacheMaintenanceSessionError::Registration(error.into())
                    })?,
                });
            }
            selected
        };
        let mut entries = Vec::new();
        for namespace in &namespaces {
            entries.extend(
                super::inventory::inventory_cache_namespace_from_home(
                    self.home,
                    self.guard.home_dir(),
                    namespace,
                    limits,
                    None,
                )?
                .into_iter()
                .map(super::inventory::PinnedCacheEntry::into_entry),
            );
        }
        Ok(entries)
    }

    /// Execute a plan using the same ownership snapshot and exclusive lock.
    pub fn execute_cleanup(
        &self,
        plan: &CleanupPlan,
        inventory_limits: CacheInventoryLimits,
        execution_limits: CacheExecutionLimits,
    ) -> Result<CacheExecutionReport, CacheMaintenanceSessionError> {
        let selected_names = plan
            .decisions()
            .iter()
            .filter(|decision| decision.will_remove())
            .map(|decision| decision.entry().namespace())
            .collect::<BTreeSet<_>>();
        let namespaces = self
            .ownership
            .namespaces()?
            .into_iter()
            .filter(|namespace| selected_names.contains(namespace.name()))
            .collect::<Vec<_>>();
        super::executor::execute_cache_cleanup_under_guard(
            self.home,
            &self.guard,
            plan,
            &namespaces,
            inventory_limits,
            execution_limits,
        )
        .map_err(Into::into)
    }
}

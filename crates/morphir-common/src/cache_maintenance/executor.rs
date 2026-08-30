mod filesystem;

use self::filesystem::{
    MaintenanceGuard, TrashRun, create_trash_run, open_maintenance_trash, remove_revalidated_entry,
    sweep_existing_trash,
};
use super::{
    CacheEntryState, CacheInventoryError, CacheInventoryLimits, CacheNamespace, CleanupPlan,
    inventory_cache_namespace,
};
use crate::home::MorphirHome;
use serde::Serialize;
use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use thiserror::Error;
use tracing::{debug, info, warn};

/// Hard removal-count and byte budgets for one cleanup execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheExecutionLimits {
    max_removals: usize,
    max_bytes: u64,
}

impl CacheExecutionLimits {
    /// Construct nonzero per-run removal and byte budgets.
    pub fn new(max_removals: usize, max_bytes: u64) -> Result<Self, CacheExecutionError> {
        if max_removals == 0 || max_bytes == 0 {
            return Err(CacheExecutionError::InvalidLimits);
        }
        Ok(Self {
            max_removals,
            max_bytes,
        })
    }
}

/// Result of revalidating and executing one planner-selected entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheExecutionDisposition {
    /// The exact revalidated entry was removed.
    Removed,
    /// The entry disappeared after inventory and required no action.
    Missing,
    /// The entry's observed bytes changed after the plan was created.
    Stale,
    /// A lease acquired after planning now protects the entry.
    ActiveLease,
    /// The path became link-like, special, or otherwise unclassified.
    Unclassified,
    /// The current ownership snapshot does not register the namespace.
    Unregistered,
    /// This and subsequent selected entries were left for a later bounded run.
    DeferredLimit,
}

/// Stable execution result for one planner-selected entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheExecutionItem {
    namespace: String,
    path: String,
    planned_bytes: u64,
    observed_bytes: Option<u64>,
    disposition: CacheExecutionDisposition,
}

impl CacheExecutionItem {
    fn new(
        namespace: impl Into<String>,
        path: impl Into<String>,
        planned_bytes: u64,
        observed_bytes: Option<u64>,
        disposition: CacheExecutionDisposition,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            path: path.into(),
            planned_bytes,
            observed_bytes,
            disposition,
        }
    }

    /// Namespace owning the selected entry.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Portable path relative to the namespace root.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Bytes recorded by the plan.
    pub fn planned_bytes(&self) -> u64 {
        self.planned_bytes
    }

    /// Bytes observed during execution, when the entry still existed.
    pub fn observed_bytes(&self) -> Option<u64> {
        self.observed_bytes
    }

    /// Outcome of executing this selected entry.
    pub fn disposition(&self) -> CacheExecutionDisposition {
        self.disposition
    }
}

/// Deterministic, serializable result of a bounded cleanup execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheExecutionReport {
    removed_bytes: u64,
    items: Vec<CacheExecutionItem>,
}

impl CacheExecutionReport {
    /// Bytes actually removed from active cache namespaces.
    pub fn removed_bytes(&self) -> u64 {
        self.removed_bytes
    }

    /// Results in the planner's stable namespace-and-path order.
    pub fn items(&self) -> &[CacheExecutionItem] {
        &self.items
    }
}

/// A fail-closed cleanup execution error.
#[derive(Debug, Error)]
pub enum CacheExecutionError {
    /// Execution budgets must both be nonzero.
    #[error("cache execution limits must be nonzero")]
    InvalidLimits,
    /// Each ownership namespace may be supplied only once.
    #[error("duplicate cache ownership namespace {namespace}")]
    DuplicateNamespace {
        /// Repeated namespace identifier.
        namespace: String,
    },
    /// Filesystem inventory failed while revalidating a selected entry.
    #[error(transparent)]
    Inventory(#[from] CacheInventoryError),
    /// A maintenance lock or trash path was replaced with an unsafe object.
    #[error("refusing to use unsafe maintenance path {path}")]
    UnsafeMaintenancePath {
        /// Path that failed the safety check.
        path: PathBuf,
    },
    /// Execution byte accounting overflowed.
    #[error("cache execution byte total exceeds the supported range")]
    ByteCountOverflow,
    /// A filesystem operation failed.
    #[error("cache cleanup failed at {path}: {source}")]
    Io {
        /// Path being operated on.
        path: PathBuf,
        /// Underlying filesystem failure.
        #[source]
        source: io::Error,
    },
}

impl CacheExecutionError {
    fn code(&self) -> &'static str {
        match self {
            Self::InvalidLimits => "invalid-limits",
            Self::DuplicateNamespace { .. } => "duplicate-namespace",
            Self::Inventory(_) => "inventory-failed",
            Self::UnsafeMaintenancePath { .. } => "unsafe-maintenance-path",
            Self::ByteCountOverflow => "byte-count-overflow",
            Self::Io { .. } => "io-failed",
        }
    }
}

/// Execute only removal decisions produced by the in-memory planner.
///
/// The executor takes the shared maintenance lock, re-inventories each selected
/// entry against the current ownership and lease registrations, refuses stale
/// or unsafe paths, and moves removals beneath Morphir Home's maintenance trash
/// before deleting them. The per-run limits make the same operation suitable
/// for manual and opportunistic automatic cleanup.
///
/// # Example
///
/// ```no_run
/// use morphir_common::cache_maintenance::{
///     CacheExecutionLimits, CacheInventoryLimits, CacheNamespace, CachePolicy, CleanupMode,
///     execute_cache_cleanup, inventory_cache_namespace, plan_cache_cleanup,
/// };
/// use morphir_common::home::MorphirHome;
/// use std::time::Duration;
///
/// let home = MorphirHome::resolve()?;
/// let downloads = CacheNamespace::new("downloads")?
///     .with_disposable("desktop.tar.gz", 1_000)?;
/// let inventory = inventory_cache_namespace(
///     &home,
///     &downloads,
///     CacheInventoryLimits::default(),
/// )?;
/// let plan = plan_cache_cleanup(
///     inventory,
///     CachePolicy::new(Duration::from_secs(30 * 24 * 60 * 60), 1_000_000_000),
///     2_000,
///     CleanupMode::Policy,
/// )?;
/// let report = execute_cache_cleanup(
///     &home,
///     &plan,
///     &[downloads],
///     CacheInventoryLimits::default(),
///     CacheExecutionLimits::new(100, 100_000_000)?,
/// )?;
/// println!("removed {} bytes", report.removed_bytes());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn execute_cache_cleanup(
    home: &MorphirHome,
    plan: &CleanupPlan,
    ownership: &[CacheNamespace],
    inventory_limits: CacheInventoryLimits,
    limits: CacheExecutionLimits,
) -> Result<CacheExecutionReport, CacheExecutionError> {
    let selected_entries = plan
        .decisions()
        .iter()
        .filter(|decision| decision.will_remove())
        .count();
    info!(
        event = "cache_cleanup_started",
        selected_entries,
        max_removals = limits.max_removals,
        max_bytes = limits.max_bytes,
        "cache cleanup started"
    );
    let result = execute_cache_cleanup_inner(home, plan, ownership, inventory_limits, limits);
    match &result {
        Ok(report) => {
            for (entry_index, item) in report.items().iter().enumerate() {
                debug!(
                    event = "cache_cleanup_entry_finished",
                    entry_index,
                    namespace = item.namespace(),
                    planned_bytes = item.planned_bytes(),
                    observed_bytes = item.observed_bytes(),
                    disposition = ?item.disposition(),
                    "cache cleanup entry finished"
                );
            }
            info!(
                event = "cache_cleanup_finished",
                selected_entries,
                result_entries = report.items().len(),
                removed_bytes = report.removed_bytes(),
                "cache cleanup finished"
            );
        }
        Err(error) => warn!(
            event = "cache_cleanup_failed",
            selected_entries,
            error_code = error.code(),
            "cache cleanup failed"
        ),
    }
    result
}

fn execute_cache_cleanup_inner(
    home: &MorphirHome,
    plan: &CleanupPlan,
    ownership: &[CacheNamespace],
    inventory_limits: CacheInventoryLimits,
    limits: CacheExecutionLimits,
) -> Result<CacheExecutionReport, CacheExecutionError> {
    let _guard = MaintenanceGuard::acquire(home)?;
    let trash = open_maintenance_trash(home)?;
    sweep_existing_trash(&trash)?;
    let inventories = inventory_namespaces(home, ownership, inventory_limits)?;
    let selected = plan
        .decisions()
        .iter()
        .filter(|decision| decision.will_remove());
    let mut items = Vec::new();
    let mut attempted = 0_usize;
    let mut budgeted_bytes = 0_u64;
    let mut removed_bytes = 0_u64;
    let mut deferred = false;
    let mut trash_run: Option<TrashRun> = None;

    for decision in selected {
        let entry = decision.entry();
        let next_budgeted_bytes = budgeted_bytes
            .checked_add(entry.bytes())
            .ok_or(CacheExecutionError::ByteCountOverflow)?;
        if deferred || attempted == limits.max_removals || next_budgeted_bytes > limits.max_bytes {
            deferred = true;
            items.push(execution_item(
                entry,
                None,
                CacheExecutionDisposition::DeferredLimit,
            ));
            continue;
        }
        attempted += 1;
        budgeted_bytes = next_budgeted_bytes;

        match revalidate_entry(&inventories, entry.namespace(), entry.path(), entry.bytes()) {
            RevalidatedEntry::Missing => items.push(execution_item(
                entry,
                None,
                CacheExecutionDisposition::Missing,
            )),
            RevalidatedEntry::ActiveLease { observed_bytes } => items.push(execution_item(
                entry,
                Some(observed_bytes),
                CacheExecutionDisposition::ActiveLease,
            )),
            RevalidatedEntry::Unclassified { observed_bytes } => items.push(execution_item(
                entry,
                Some(observed_bytes),
                CacheExecutionDisposition::Unclassified,
            )),
            RevalidatedEntry::Unregistered => items.push(execution_item(
                entry,
                None,
                CacheExecutionDisposition::Unregistered,
            )),
            RevalidatedEntry::Stale { observed_bytes } => items.push(execution_item(
                entry,
                Some(observed_bytes),
                CacheExecutionDisposition::Stale,
            )),
            RevalidatedEntry::Ready => {
                if trash_run.is_none() {
                    trash_run = Some(create_trash_run(&trash)?);
                }
                let run = trash_run.as_ref().expect("trash run was just initialized");
                remove_revalidated_entry(home, entry.namespace(), entry.path(), run, items.len())?;
                removed_bytes = removed_bytes
                    .checked_add(entry.bytes())
                    .ok_or(CacheExecutionError::ByteCountOverflow)?;
                items.push(execution_item(
                    entry,
                    Some(entry.bytes()),
                    CacheExecutionDisposition::Removed,
                ));
            }
        }
    }

    if let Some(run) = trash_run {
        run.finish()?;
    }
    Ok(CacheExecutionReport {
        removed_bytes,
        items,
    })
}

fn execution_item(
    entry: &super::CacheEntry,
    observed_bytes: Option<u64>,
    disposition: CacheExecutionDisposition,
) -> CacheExecutionItem {
    CacheExecutionItem::new(
        entry.namespace(),
        entry.path(),
        entry.bytes(),
        observed_bytes,
        disposition,
    )
}

enum RevalidatedEntry {
    Missing,
    ActiveLease { observed_bytes: u64 },
    Unclassified { observed_bytes: u64 },
    Unregistered,
    Stale { observed_bytes: u64 },
    Ready,
}

fn inventory_namespaces(
    home: &MorphirHome,
    ownership: &[CacheNamespace],
    limits: CacheInventoryLimits,
) -> Result<BTreeMap<String, Vec<super::CacheEntry>>, CacheExecutionError> {
    let mut inventories = BTreeMap::new();
    for namespace in ownership {
        if inventories.contains_key(namespace.name()) {
            return Err(CacheExecutionError::DuplicateNamespace {
                namespace: namespace.name().to_owned(),
            });
        }
        inventories.insert(
            namespace.name().to_owned(),
            inventory_cache_namespace(home, namespace, limits)?,
        );
    }
    Ok(inventories)
}

fn revalidate_entry(
    inventories: &BTreeMap<String, Vec<super::CacheEntry>>,
    namespace: &str,
    path: &str,
    planned_bytes: u64,
) -> RevalidatedEntry {
    let Some(inventory) = inventories.get(namespace) else {
        return RevalidatedEntry::Unregistered;
    };
    if let Some(observed) = inventory.iter().find(|entry| entry.path() == path) {
        return match observed.state() {
            CacheEntryState::Disposable { .. } if observed.bytes() == planned_bytes => {
                RevalidatedEntry::Ready
            }
            CacheEntryState::Disposable { .. } => RevalidatedEntry::Stale {
                observed_bytes: observed.bytes(),
            },
            CacheEntryState::ActiveLease { .. } => RevalidatedEntry::ActiveLease {
                observed_bytes: observed.bytes(),
            },
            CacheEntryState::Unclassified => RevalidatedEntry::Unclassified {
                observed_bytes: observed.bytes(),
            },
        };
    }
    let unsafe_ancestor = inventory.iter().find(|entry| {
        entry.state() == CacheEntryState::Unclassified
            && path
                .strip_prefix(entry.path())
                .is_some_and(|rest| rest.starts_with('/'))
    });
    match unsafe_ancestor {
        Some(entry) => RevalidatedEntry::Unclassified {
            observed_bytes: entry.bytes(),
        },
        None => RevalidatedEntry::Missing,
    }
}

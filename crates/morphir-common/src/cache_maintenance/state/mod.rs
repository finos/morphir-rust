mod model;
mod persistence;

use std::path::PathBuf;
use thiserror::Error;

pub use model::{
    AutomaticCacheCleanupDecision, CacheCleanupCursor, CacheMaintenanceState,
    automatic_cache_cleanup_decision,
};
pub use persistence::{load_cache_maintenance_state, save_cache_maintenance_state};

/// Exclusive automatic-maintenance transaction spanning state load, cleanup,
/// and durable state replacement.
///
/// Keeping this value alive prevents CLI and Desktop processes from acting on
/// the same stale continuation. Manual cleanup remains available through
/// [`super::execute_cache_cleanup`].
///
/// ```no_run
/// use morphir_common::cache_maintenance::{
///     AutomaticCacheMaintenanceTransaction, CacheMaintenanceState,
/// };
/// use morphir_common::home::MorphirHome;
///
/// let home = MorphirHome::resolve()?;
/// let transaction = AutomaticCacheMaintenanceTransaction::begin(&home)?;
/// let next_state = transaction.state().clone().completed(1_000);
/// // Inventory, plan, and call transaction.execute_cleanup(...) here.
/// transaction.finish(next_state)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct AutomaticCacheMaintenanceTransaction<'home> {
    home: &'home crate::home::MorphirHome,
    state: CacheMaintenanceState,
    _guard: super::executor::MaintenanceGuard,
}

impl<'home> AutomaticCacheMaintenanceTransaction<'home> {
    /// Acquire exclusive suite coordination and load the latest durable state.
    pub fn begin(
        home: &'home crate::home::MorphirHome,
    ) -> Result<Self, CacheMaintenanceStateError> {
        let guard = super::executor::MaintenanceGuard::acquire(home)
            .map_err(CacheMaintenanceStateError::Coordination)?;
        let state = persistence::load_cache_maintenance_state(home)?;
        Ok(Self {
            home,
            state,
            _guard: guard,
        })
    }

    /// State loaded after exclusive coordination was acquired.
    pub fn state(&self) -> &CacheMaintenanceState {
        &self.state
    }

    /// Execute a cleanup plan without reacquiring the transaction's lock.
    pub fn execute_cleanup(
        &self,
        plan: &super::CleanupPlan,
        ownership: &[super::CacheNamespace],
        inventory_limits: super::CacheInventoryLimits,
        limits: super::CacheExecutionLimits,
    ) -> Result<super::CacheExecutionReport, super::CacheExecutionError> {
        super::executor::execute_cache_cleanup_under_guard(
            self.home,
            plan,
            ownership,
            inventory_limits,
            limits,
        )
    }

    /// Durably replace state and release exclusive suite coordination.
    pub fn finish(self, state: CacheMaintenanceState) -> Result<(), CacheMaintenanceStateError> {
        persistence::save_cache_maintenance_state_under_guard(self.home, &state)
    }
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

//! Shared, deterministic policy planning for disposable Morphir caches.
//!
//! Components inventory only entries they own and classify anything else as
//! unclassified. The planner is deliberately free of filesystem side effects
//! so CLI and Desktop adapters can present a dry run and execute exactly the
//! same decisions.
//!
//! ```
//! use morphir_common::cache_maintenance::{
//!     CacheEntry, CachePolicy, CleanupMode, plan_cache_cleanup,
//! };
//! use std::time::Duration;
//!
//! let entries = vec![CacheEntry::disposable("downloads", "old.pkg", 12, 1)?];
//! let plan = plan_cache_cleanup(
//!     entries,
//!     CachePolicy::new(Duration::from_secs(30), 1024),
//!     60,
//!     CleanupMode::Policy,
//! )?;
//! assert_eq!(plan.reclaimable_bytes(), 12);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod inventory;
mod model;
mod planner;

pub use inventory::{
    CacheInventoryError, CacheInventoryLimits, CacheNamespace, CacheRegistrationError,
    inventory_cache_namespace,
};
pub use model::{
    CacheDecision, CacheDecisionReason, CacheEntry, CacheEntryState, CacheModelError, CachePolicy,
    CleanupMode, CleanupPlan,
};
pub use planner::{CachePlanError, plan_cache_cleanup};

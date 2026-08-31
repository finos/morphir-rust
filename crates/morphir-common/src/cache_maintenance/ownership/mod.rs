//! Durable ownership declarations for disposable Morphir caches.
//!
//! Producers hold [`super::CacheMutationGuard`] while writing or using cache
//! content, close their content handles, and finish the guard by publishing
//! ownership. The consuming transition keeps cleanup excluded until the
//! registry update is durable. An interrupted first-time producer leaves
//! unknown content that cleanup preserves.
//!
//! ```no_run
//! use morphir_common::cache_maintenance::CacheMutationGuard;
//! use morphir_common::home::MorphirHome;
//!
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let home = MorphirHome::resolve()?;
//! let mutation = CacheMutationGuard::acquire(&home)?;
//! // Write and atomically publish cache/downloads/desktop/1.2.3.pkg here.
//! mutation.finish_with_ownership(
//!     "downloads",
//!     "desktop/1.2.3.pkg",
//!     1_735_689_600,
//! )?;
//! # Ok(())
//! # }
//! ```

mod model;
mod persistence;

pub use model::{CacheOwnershipRegistry, CacheOwnershipRegistryError};
pub(crate) use persistence::load_cache_ownership_registry_under_guard;
pub use persistence::{
    CacheOwnershipHandoffError, CacheOwnershipPersistenceError, load_cache_ownership_registry,
};

//! Durable ownership declarations for disposable Morphir caches.
//!
//! Producers begin a [`CacheOwnershipMutationGuard`] for a specific identity
//! before writing cache content, close their content handles, and finish the
//! guard by publishing ownership. Beginning durably invalidates every prior
//! registration that overlaps the mutation path; finishing keeps cleanup
//! excluded until the new registration is durable. An interrupted producer
//! therefore leaves protected, unknown content rather than stale disposable
//! ownership.
//!
//! ```no_run
//! use morphir_common::cache_maintenance::CacheOwnershipMutationGuard;
//! use morphir_common::home::MorphirHome;
//!
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let home = MorphirHome::resolve()?;
//! let mutation = CacheOwnershipMutationGuard::begin(
//!     &home,
//!     "downloads",
//!     "desktop/1.2.3.pkg",
//! )?;
//! // Write and atomically publish cache/downloads/desktop/1.2.3.pkg here.
//! mutation.finish(1_735_689_600)?;
//! # Ok(())
//! # }
//! ```

mod model;
mod persistence;

pub use model::{CacheOwnershipRegistry, CacheOwnershipRegistryError};
pub(crate) use persistence::load_cache_ownership_registry_under_guard;
pub(crate) use persistence::save_cache_ownership_registry_under_guard;
pub use persistence::{
    CacheOwnershipHandoffError, CacheOwnershipMutationGuard, CacheOwnershipPersistenceError,
    load_cache_ownership_registry,
};

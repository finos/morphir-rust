//! Durable ownership declarations for disposable Morphir caches.
//!
//! Producers hold [`super::CacheMutationGuard`] while writing or using cache
//! content, release that shared lease, and only then register the completed
//! entry. Registering after the write is fail-safe: an interrupted producer
//! leaves unknown content that cleanup preserves.
//!
//! ```no_run
//! use morphir_common::cache_maintenance::{
//!     CacheMutationGuard, register_cache_ownership,
//! };
//! use morphir_common::home::MorphirHome;
//!
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let home = MorphirHome::resolve()?;
//! let mutation = CacheMutationGuard::acquire(&home)?;
//! // Write and atomically publish cache/downloads/desktop/1.2.3.pkg here.
//! drop(mutation);
//! register_cache_ownership(
//!     &home,
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
    CacheOwnershipPersistenceError, load_cache_ownership_registry, register_cache_ownership,
    unregister_cache_ownership,
};

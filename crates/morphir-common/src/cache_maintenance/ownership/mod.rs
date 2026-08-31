mod model;
mod persistence;

pub use model::{CacheOwnershipRegistry, CacheOwnershipRegistryError};
pub(crate) use persistence::load_cache_ownership_registry_under_guard;
pub use persistence::{
    CacheOwnershipPersistenceError, load_cache_ownership_registry, register_cache_ownership,
    unregister_cache_ownership,
};

use super::super::inventory::portable_comparison_key;
use super::super::{CacheNamespace, CacheRegistrationError};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use thiserror::Error;

const CACHE_OWNERSHIP_SCHEMA_VERSION: u32 = 1;

/// Invalid trusted ownership metadata supplied by a cache producer.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CacheOwnershipRegistryError {
    /// Namespace or entry identity is invalid, or entries overlap.
    #[error(transparent)]
    InvalidRegistration(#[from] CacheRegistrationError),
    /// A persisted registry declared the same portable entry more than once.
    #[error("duplicate cache ownership entry {namespace}/{path}")]
    DuplicateEntry {
        /// Owning cache namespace.
        namespace: String,
        /// Portable path relative to the namespace root.
        path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OwnedCacheEntry {
    namespace: String,
    path: String,
    last_used: u64,
}

/// Versioned trusted declarations of disposable cache content.
///
/// The registry contains only entries explicitly registered by their producer.
/// Filesystem content absent from this value remains unclassified and is never
/// selected for cleanup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheOwnershipRegistry {
    entries: BTreeMap<(String, String), OwnedCacheEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CacheOwnershipRegistryRef<'a> {
    schema_version: u32,
    entries: Vec<&'a OwnedCacheEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CacheOwnershipRegistryWire {
    schema_version: u32,
    #[serde(default)]
    entries: Vec<OwnedCacheEntry>,
}

impl Serialize for CacheOwnershipRegistry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        CacheOwnershipRegistryRef {
            schema_version: CACHE_OWNERSHIP_SCHEMA_VERSION,
            entries: self.entries.values().collect(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CacheOwnershipRegistry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CacheOwnershipRegistryWire::deserialize(deserializer)?;
        if wire.schema_version != CACHE_OWNERSHIP_SCHEMA_VERSION {
            return Err(serde::de::Error::custom(format!(
                "unsupported cache ownership schema version {}",
                wire.schema_version
            )));
        }
        let mut registry = Self::default();
        for entry in wire.entries {
            registry
                .insert(entry, false)
                .map_err(serde::de::Error::custom)?;
        }
        Ok(registry)
    }
}

impl CacheOwnershipRegistry {
    /// Number of explicitly owned cache entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no cache producer has registered disposable content.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Register or refresh one disposable entry after its cache mutation ends.
    pub fn register_disposable(
        &mut self,
        namespace: impl Into<String>,
        path: impl Into<String>,
        last_used: u64,
    ) -> Result<(), CacheOwnershipRegistryError> {
        self.insert(
            OwnedCacheEntry {
                namespace: namespace.into(),
                path: path.into(),
                last_used,
            },
            true,
        )
    }

    /// Stop declaring ownership of one portable cache entry.
    ///
    /// Missing entries are a successful no-op. The cache content itself is not
    /// modified and therefore becomes protected, unclassified content.
    pub fn unregister(
        &mut self,
        namespace: &str,
        path: &str,
    ) -> Result<bool, CacheOwnershipRegistryError> {
        CacheNamespace::new(namespace)?.with_disposable(path, 0)?;
        Ok(self
            .entries
            .remove(&comparison_key(namespace, path))
            .is_some())
    }

    /// Convert trusted declarations into deterministic inventory namespaces.
    pub fn namespaces(&self) -> Result<Vec<CacheNamespace>, CacheOwnershipRegistryError> {
        let mut namespaces = BTreeMap::<String, CacheNamespace>::new();
        for entry in self.entries.values() {
            let namespace = namespaces
                .remove(&entry.namespace)
                .unwrap_or(CacheNamespace::new(&entry.namespace)?);
            namespaces.insert(
                entry.namespace.clone(),
                namespace.with_disposable(&entry.path, entry.last_used)?,
            );
        }
        Ok(namespaces.into_values().collect())
    }

    fn insert(
        &mut self,
        entry: OwnedCacheEntry,
        replace_exact: bool,
    ) -> Result<(), CacheOwnershipRegistryError> {
        let candidate =
            CacheNamespace::new(&entry.namespace)?.with_disposable(&entry.path, entry.last_used)?;
        let key = comparison_key(&entry.namespace, &entry.path);
        if let Some(existing) = self.entries.get_mut(&key) {
            if replace_exact {
                existing.last_used = existing.last_used.max(entry.last_used);
                return Ok(());
            }
            return Err(CacheOwnershipRegistryError::DuplicateEntry {
                namespace: entry.namespace,
                path: entry.path,
            });
        }

        let mut namespace = CacheNamespace::new(&entry.namespace)?;
        for existing in self
            .entries
            .values()
            .filter(|existing| portable_comparison_key(&existing.namespace) == key.0)
        {
            namespace = namespace.with_disposable(&existing.path, existing.last_used)?;
        }
        namespace.with_disposable(&entry.path, entry.last_used)?;
        drop(candidate);
        self.entries.insert(key, entry);
        Ok(())
    }
}

fn comparison_key(namespace: &str, path: &str) -> (String, String) {
    (
        portable_comparison_key(namespace),
        portable_comparison_key(path),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_refreshes_exact_entries_and_rejects_overlaps() {
        let mut registry = CacheOwnershipRegistry::default();
        registry
            .register_disposable("downloads", "desktop/1.0/pkg", 10)
            .unwrap();
        registry
            .register_disposable("downloads", "desktop/1.0/pkg", 20)
            .unwrap();

        assert_eq!(registry.len(), 1);
        let serialized = serde_json::to_value(&registry).unwrap();
        assert_eq!(serialized["entries"][0]["lastUsed"], 20);

        let error = registry
            .register_disposable("downloads", "desktop", 30)
            .unwrap_err();
        assert!(matches!(
            error,
            CacheOwnershipRegistryError::InvalidRegistration(
                CacheRegistrationError::OverlappingEntries { .. }
            )
        ));
    }

    #[test]
    fn registration_refreshes_never_move_last_used_backwards() {
        let mut registry = CacheOwnershipRegistry::default();
        registry
            .register_disposable("downloads", "desktop/1.0/pkg", 20)
            .unwrap();
        registry
            .register_disposable("downloads", "desktop/1.0/pkg", 10)
            .unwrap();

        let serialized = serde_json::to_value(&registry).unwrap();
        assert_eq!(serialized["entries"][0]["lastUsed"], 20);
    }

    #[test]
    fn serde_is_stable_strict_and_rejects_portable_duplicates() {
        let json = r#"{
            "schemaVersion": 1,
            "entries": [
                {"namespace":"downloads","path":"CAF\u00c9/pkg","lastUsed":1},
                {"namespace":"downloads","path":"cafe\u0301/pkg","lastUsed":2}
            ]
        }"#;
        let error = serde_json::from_str::<CacheOwnershipRegistry>(json).unwrap_err();
        assert!(error.to_string().contains("duplicate"));

        let unknown = r#"{"schemaVersion":1,"entries":[],"extra":true}"#;
        assert!(serde_json::from_str::<CacheOwnershipRegistry>(unknown).is_err());
        let future = r#"{"schemaVersion":2,"entries":[]}"#;
        assert!(serde_json::from_str::<CacheOwnershipRegistry>(future).is_err());
    }

    #[test]
    fn unregister_is_idempotent_and_preserves_other_entries() {
        let mut registry = CacheOwnershipRegistry::default();
        registry
            .register_disposable("downloads", "one.pkg", 1)
            .unwrap();
        registry
            .register_disposable("indexes", "catalog.json", 2)
            .unwrap();

        assert!(registry.unregister("downloads", "one.pkg").unwrap());
        assert!(!registry.unregister("downloads", "one.pkg").unwrap());
        assert_eq!(registry.len(), 1);
    }
}

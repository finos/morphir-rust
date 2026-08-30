use super::super::{CacheEntry, CacheModelError};
use std::collections::BTreeMap;
use thiserror::Error;
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

/// Invalid ownership metadata supplied by a cache namespace.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CacheRegistrationError {
    /// Namespace or entry identity violates the portable grammar.
    #[error(transparent)]
    InvalidIdentity(#[from] CacheModelError),
    /// Registered entries may not duplicate or contain one another.
    #[error("cache entries overlap: {first} and {second}")]
    OverlappingEntries {
        /// First registered portable path.
        first: String,
        /// Second registered portable path.
        second: String,
    },
}

/// Trusted ownership declarations for one Morphir Home cache namespace.
#[derive(Debug, Clone)]
pub struct CacheNamespace {
    pub(super) name: String,
    pub(super) entries: BTreeMap<String, CacheEntry>,
}

impl CacheNamespace {
    /// Register a namespace rooted at `<MORPHIR_HOME>/cache/<name>`.
    pub fn new(name: impl Into<String>) -> Result<Self, CacheRegistrationError> {
        let name = name.into();
        CacheEntry::unclassified(name.clone(), "identity-probe", 0)?;
        Ok(Self {
            name,
            entries: BTreeMap::new(),
        })
    }

    /// Register one owned disposable entry and its last-use timestamp.
    pub fn with_disposable(
        mut self,
        path: impl Into<String>,
        last_used: u64,
    ) -> Result<Self, CacheRegistrationError> {
        let path = path.into();
        let entry = CacheEntry::disposable(self.name.clone(), path.clone(), 0, last_used)?;
        self.insert(path, entry)?;
        Ok(self)
    }

    /// Register one owned entry currently protected by an active lease.
    pub fn with_lease(
        mut self,
        path: impl Into<String>,
        last_used: u64,
    ) -> Result<Self, CacheRegistrationError> {
        let path = path.into();
        let entry = CacheEntry::leased(self.name.clone(), path.clone(), 0, last_used)?;
        self.insert(path, entry)?;
        Ok(self)
    }

    /// Stable namespace owner identifier.
    pub fn name(&self) -> &str {
        &self.name
    }

    fn insert(&mut self, path: String, entry: CacheEntry) -> Result<(), CacheRegistrationError> {
        if let Some(other) = self
            .entries
            .keys()
            .find(|other| paths_overlap(other, &path))
        {
            return Err(CacheRegistrationError::OverlappingEntries {
                first: other.clone(),
                second: path,
            });
        }
        self.entries.insert(path, entry);
        Ok(())
    }
}

fn paths_overlap(first: &str, second: &str) -> bool {
    let first = portable_comparison_key(first);
    let second = portable_comparison_key(second);
    first == second
        || first
            .strip_prefix(&second)
            .is_some_and(|rest| rest.starts_with('/'))
        || second
            .strip_prefix(&first)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn portable_comparison_key(path: &str) -> String {
    path.nfc()
        .collect::<String>()
        .as_str()
        .case_fold()
        .collect::<String>()
        .nfc()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{CacheNamespace, CacheRegistrationError, paths_overlap};

    #[test]
    fn registration_rejects_duplicate_nested_and_case_folded_ownership() {
        let nested = CacheNamespace::new("downloads")
            .unwrap()
            .with_disposable("packages/tool", 1)
            .unwrap()
            .with_disposable("packages/tool/nested", 2)
            .unwrap_err();
        assert!(matches!(
            nested,
            CacheRegistrationError::OverlappingEntries { .. }
        ));
        let case_folded = CacheNamespace::new("downloads")
            .unwrap()
            .with_disposable("artifact", 1)
            .unwrap()
            .with_lease("ARTIFACT", 2)
            .unwrap_err();
        assert!(matches!(
            case_folded,
            CacheRegistrationError::OverlappingEntries { .. }
        ));
        let normalized = CacheNamespace::new("downloads")
            .unwrap()
            .with_disposable("caf\u{e9}/artifact", 1)
            .unwrap()
            .with_lease("cafe\u{301}/artifact", 2)
            .unwrap_err();
        assert!(matches!(
            normalized,
            CacheRegistrationError::OverlappingEntries { .. }
        ));
        assert!(paths_overlap("a", "a/b"));
        assert!(!paths_overlap("a", "ab"));
    }
}

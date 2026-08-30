use serde::{Deserialize, Deserializer, Serialize};
use std::time::Duration;
use thiserror::Error;

const MAX_NAMESPACE_BYTES: usize = 64;
const MAX_ENTRY_PATH_BYTES: usize = 1024;
const MAX_ENTRY_SEGMENT_UNITS: usize = 255;

/// A malformed namespace or entry identity supplied to the maintenance engine.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CacheModelError {
    /// Namespace names use a bounded lowercase portable grammar.
    #[error("cache namespace must match [a-z0-9][a-z0-9-]* and be at most 64 bytes")]
    InvalidNamespace,
    /// Entry paths must be bounded, relative, portable paths.
    #[error("cache entry path must be a bounded portable relative path")]
    InvalidEntryPath,
}

/// The ownership and lease classification of one inventoried cache entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CacheEntryState {
    /// The owning component permits policy-based removal.
    Disposable {
        /// Unix timestamp in seconds recorded by the owning component.
        #[serde(rename = "lastUsed")]
        last_used: u64,
    },
    /// A running operation currently protects the otherwise disposable entry.
    ActiveLease {
        /// Unix timestamp in seconds recorded by the owning component.
        #[serde(rename = "lastUsed")]
        last_used: u64,
    },
    /// The namespace owner did not positively classify this entry.
    Unclassified,
}

/// One cache entry observed beneath a registered namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheEntry {
    namespace: String,
    path: String,
    bytes: u64,
    state: CacheEntryState,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CacheEntryWire {
    namespace: String,
    path: String,
    bytes: u64,
    state: CacheEntryState,
}

impl<'de> Deserialize<'de> for CacheEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CacheEntryWire::deserialize(deserializer)?;
        Self::new(wire.namespace, wire.path, wire.bytes, wire.state)
            .map_err(serde::de::Error::custom)
    }
}

impl CacheEntry {
    /// Construct an owned entry that policy may remove.
    pub fn disposable(
        namespace: impl Into<String>,
        path: impl Into<String>,
        bytes: u64,
        last_used: u64,
    ) -> Result<Self, CacheModelError> {
        Self::new(
            namespace.into(),
            path.into(),
            bytes,
            CacheEntryState::Disposable { last_used },
        )
    }

    /// Construct an owned entry protected by an active operation lease.
    pub fn leased(
        namespace: impl Into<String>,
        path: impl Into<String>,
        bytes: u64,
        last_used: u64,
    ) -> Result<Self, CacheModelError> {
        Self::new(
            namespace.into(),
            path.into(),
            bytes,
            CacheEntryState::ActiveLease { last_used },
        )
    }

    /// Construct an entry the namespace owner cannot safely classify.
    pub fn unclassified(
        namespace: impl Into<String>,
        path: impl Into<String>,
        bytes: u64,
    ) -> Result<Self, CacheModelError> {
        Self::new(
            namespace.into(),
            path.into(),
            bytes,
            CacheEntryState::Unclassified,
        )
    }

    fn new(
        namespace: String,
        path: String,
        bytes: u64,
        state: CacheEntryState,
    ) -> Result<Self, CacheModelError> {
        if !valid_namespace(&namespace) {
            return Err(CacheModelError::InvalidNamespace);
        }
        if !valid_entry_path(&path) {
            return Err(CacheModelError::InvalidEntryPath);
        }
        Ok(Self {
            namespace,
            path,
            bytes,
            state,
        })
    }

    /// Registered owner of this entry's namespace.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Portable path relative to the namespace root.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Observed size of the entry without following link-like objects.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Ownership and lease classification used by the planner.
    pub fn state(&self) -> CacheEntryState {
        self.state
    }

    pub(crate) fn last_used(&self) -> Option<u64> {
        match self.state {
            CacheEntryState::Disposable { last_used }
            | CacheEntryState::ActiveLease { last_used } => Some(last_used),
            CacheEntryState::Unclassified => None,
        }
    }
}

/// Time and size limits applied by policy cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachePolicy {
    max_age_seconds: u64,
    max_size_bytes: u64,
}

impl CachePolicy {
    /// Construct a policy from an age limit and target size.
    pub fn new(max_age: Duration, max_size_bytes: u64) -> Self {
        Self {
            max_age_seconds: max_age.as_secs(),
            max_size_bytes,
        }
    }

    /// Maximum permitted idle age in seconds.
    pub fn max_age_seconds(self) -> u64 {
        self.max_age_seconds
    }

    /// Target size for known entries after cleanup.
    pub fn max_size_bytes(self) -> u64 {
        self.max_size_bytes
    }
}

/// Whether cleanup follows configured policy or selects every disposable entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupMode {
    /// Apply age first, then least-recently-used size eviction.
    Policy,
    /// Select every owned disposable entry.
    All,
}

/// Stable explanation for a cleanup decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheDecisionReason {
    /// Entry exceeds the configured idle age.
    Expired,
    /// Entry is selected by least-recently-used size eviction.
    SizeLimit,
    /// The user requested every disposable entry.
    RemoveAll,
    /// Entry fits within the configured policy.
    WithinPolicy,
    /// A running operation protects the entry.
    ActiveLease,
    /// The owner did not positively classify the entry.
    Unclassified,
}

impl CacheDecisionReason {
    pub(crate) fn removes(self) -> bool {
        matches!(self, Self::Expired | Self::SizeLimit | Self::RemoveAll)
    }
}

/// The deterministic decision for one inventoried entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheDecision {
    entry: CacheEntry,
    reason: CacheDecisionReason,
}

impl CacheDecision {
    pub(crate) fn new(entry: CacheEntry, reason: CacheDecisionReason) -> Self {
        Self { entry, reason }
    }

    /// The entry evaluated by this decision.
    pub fn entry(&self) -> &CacheEntry {
        &self.entry
    }

    /// Stable reason explaining the outcome.
    pub fn reason(&self) -> CacheDecisionReason {
        self.reason
    }

    /// Whether execution should remove this entry.
    pub fn will_remove(&self) -> bool {
        self.reason.removes()
    }
}

/// A side-effect-free, output-only cleanup plan suitable for dry runs and execution.
///
/// Plans deliberately support serialization but not deserialization. Execute the
/// in-memory value returned by the planner rather than trusting decisions supplied
/// by an external JSON document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupPlan {
    policy: CachePolicy,
    mode: CleanupMode,
    known_bytes_before: u64,
    known_bytes_after: u64,
    unclassified_bytes: u64,
    reclaimable_bytes: u64,
    decisions: Vec<CacheDecision>,
}

impl CleanupPlan {
    pub(crate) fn new(
        policy: CachePolicy,
        mode: CleanupMode,
        known_bytes_before: u64,
        known_bytes_after: u64,
        unclassified_bytes: u64,
        reclaimable_bytes: u64,
        decisions: Vec<CacheDecision>,
    ) -> Self {
        Self {
            policy,
            mode,
            known_bytes_before,
            known_bytes_after,
            unclassified_bytes,
            reclaimable_bytes,
            decisions,
        }
    }

    /// Known owned bytes before executing the plan.
    pub fn known_bytes_before(&self) -> u64 {
        self.known_bytes_before
    }

    /// Known owned bytes expected to remain after executing the plan.
    pub fn known_bytes_after(&self) -> u64 {
        self.known_bytes_after
    }

    /// Bytes belonging to entries the engine must not remove.
    pub fn unclassified_bytes(&self) -> u64 {
        self.unclassified_bytes
    }

    /// Bytes selected for removal.
    pub fn reclaimable_bytes(&self) -> u64 {
        self.reclaimable_bytes
    }

    /// Decisions sorted by namespace and portable entry path.
    pub fn decisions(&self) -> &[CacheDecision] {
        &self.decisions
    }

    /// Find one decision by its stable entry identity.
    pub fn decision(&self, namespace: &str, path: &str) -> Option<&CacheDecision> {
        self.decisions
            .iter()
            .find(|decision| decision.entry.namespace == namespace && decision.entry.path == path)
    }
}

fn valid_namespace(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_NAMESPACE_BYTES
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_entry_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ENTRY_PATH_BYTES
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value.contains(':')
        && value.split('/').all(valid_entry_segment)
}

fn valid_entry_segment(segment: &str) -> bool {
    let windows_stem = segment
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let windows_reserved = matches!(
        windows_stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || windows_stem
        .strip_prefix("COM")
        .is_some_and(is_windows_device_number)
        || windows_stem
            .strip_prefix("LPT")
            .is_some_and(is_windows_device_number);

    !segment.is_empty()
        && segment.len() <= MAX_ENTRY_SEGMENT_UNITS
        && segment.encode_utf16().count() <= MAX_ENTRY_SEGMENT_UNITS
        && !matches!(segment, "." | "..")
        && !segment.ends_with(['.', ' '])
        && !segment.chars().any(|character| {
            character.is_control() || matches!(character, '<' | '>' | '"' | '|' | '?' | '*')
        })
        && !windows_reserved
}

fn is_windows_device_number(number: &str) -> bool {
    matches!(number, "¹" | "²" | "³")
        || (number.len() == 1 && matches!(number.as_bytes()[0], b'1'..=b'9'))
}

#[cfg(test)]
mod tests {
    use super::{CacheEntry, CacheModelError};

    #[test]
    fn portable_identities_reject_traversal_and_platform_specific_paths() {
        for path in [
            "",
            "/absolute",
            "../escape",
            "nested/../escape",
            r"C:\escape",
            "C:/escape",
            "CON",
            "con.txt",
            "nested/LPT9.log",
            "COM¹.exe",
            "foo.",
            "foo ",
            "contains<angle",
            "contains>angle",
            "contains\"quote",
            "contains|pipe",
            "contains?question",
            "contains*star",
        ] {
            assert_eq!(
                CacheEntry::disposable("downloads", path, 1, 1),
                Err(CacheModelError::InvalidEntryPath)
            );
        }
        assert_eq!(
            CacheEntry::disposable("Desktop", "entry", 1, 1),
            Err(CacheModelError::InvalidNamespace)
        );
    }

    #[test]
    fn deserialization_cannot_bypass_entry_identity_validation() {
        let error = serde_json::from_value::<CacheEntry>(serde_json::json!({
            "namespace": "downloads",
            "path": "../outside",
            "bytes": 1,
            "state": { "kind": "disposable", "lastUsed": 1 }
        }))
        .unwrap_err();

        assert!(error.to_string().contains("portable relative path"));
    }
}

use super::super::CacheModelError;
use super::CacheMaintenanceStateError;
use serde::{Deserialize, Deserializer, Serialize};
use std::time::Duration;

const CACHE_MAINTENANCE_STATE_SCHEMA_VERSION: u32 = 1;
/// A durable continuation position in deterministic namespace/path order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheCleanupCursor {
    namespace: String,
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CacheCleanupCursorWire {
    namespace: String,
    path: String,
}

impl<'de> Deserialize<'de> for CacheCleanupCursor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CacheCleanupCursorWire::deserialize(deserializer)?;
        Self::new(wire.namespace, wire.path).map_err(serde::de::Error::custom)
    }
}

impl CacheCleanupCursor {
    /// Construct a cursor from a registered namespace and portable entry path.
    pub fn new(
        namespace: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<Self, CacheModelError> {
        let namespace = namespace.into();
        let path = path.into();
        super::super::CacheEntry::unclassified(namespace.clone(), path.clone(), 0)?;
        Ok(Self { namespace, path })
    }

    /// Registered namespace of the next entry to consider.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Portable path of the next entry to consider.
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// Durable state shared by CLI and Desktop automatic cache maintenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheMaintenanceState {
    schema_version: u32,
    last_successful_automatic_run: Option<u64>,
    continuation: Option<CacheCleanupCursor>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CacheMaintenanceStateWire {
    schema_version: u32,
    #[serde(default)]
    last_successful_automatic_run: Option<u64>,
    #[serde(default)]
    continuation: Option<CacheCleanupCursor>,
}

impl<'de> Deserialize<'de> for CacheMaintenanceState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CacheMaintenanceStateWire::deserialize(deserializer)?;
        if wire.schema_version != CACHE_MAINTENANCE_STATE_SCHEMA_VERSION {
            return Err(serde::de::Error::custom(format!(
                "unsupported cache maintenance state schema version {}",
                wire.schema_version
            )));
        }
        Ok(Self {
            schema_version: wire.schema_version,
            last_successful_automatic_run: wire.last_successful_automatic_run,
            continuation: wire.continuation,
        })
    }
}

impl Default for CacheMaintenanceState {
    fn default() -> Self {
        Self {
            schema_version: CACHE_MAINTENANCE_STATE_SCHEMA_VERSION,
            last_successful_automatic_run: None,
            continuation: None,
        }
    }
}

impl CacheMaintenanceState {
    /// Timestamp of the last automatic pass that reached the end of inventory.
    pub fn last_successful_automatic_run(&self) -> Option<u64> {
        self.last_successful_automatic_run
    }

    /// Next deterministic inventory position after a bounded partial pass.
    pub fn continuation(&self) -> Option<&CacheCleanupCursor> {
        self.continuation.as_ref()
    }

    /// Record a complete automatic pass and clear any continuation.
    #[must_use]
    pub fn completed(mut self, completed_at: u64) -> Self {
        self.last_successful_automatic_run = Some(completed_at);
        self.continuation = None;
        self
    }

    /// Record a bounded partial pass without advancing the successful-run time.
    #[must_use]
    pub fn continued(mut self, cursor: CacheCleanupCursor) -> Self {
        self.continuation = Some(cursor);
        self
    }
}

/// Whether an automatic cleanup opportunity should run now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AutomaticCacheCleanupDecision {
    /// No completed pass exists, the interval elapsed, or continuation remains.
    Due,
    /// The most recent complete pass still falls within the configured interval.
    Deferred {
        /// Earliest Unix timestamp at which another complete pass is due.
        next_run: u64,
    },
}

/// Evaluate interval gating without changing durable state.
///
/// ```
/// use morphir_common::cache_maintenance::{
///     AutomaticCacheCleanupDecision, CacheMaintenanceState,
///     automatic_cache_cleanup_decision,
/// };
/// use std::time::Duration;
///
/// let state = CacheMaintenanceState::default().completed(1_000);
/// let decision = automatic_cache_cleanup_decision(
///     &state,
///     1_100,
///     Duration::from_secs(200),
/// )?;
/// assert_eq!(
///     decision,
///     AutomaticCacheCleanupDecision::Deferred { next_run: 1_200 }
/// );
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn automatic_cache_cleanup_decision(
    state: &CacheMaintenanceState,
    now: u64,
    interval: Duration,
) -> Result<AutomaticCacheCleanupDecision, CacheMaintenanceStateError> {
    if interval.as_secs() == 0 {
        return Err(CacheMaintenanceStateError::InvalidInterval);
    }
    if state.continuation.is_some() {
        return Ok(AutomaticCacheCleanupDecision::Due);
    }
    let Some(last_run) = state.last_successful_automatic_run else {
        return Ok(AutomaticCacheCleanupDecision::Due);
    };
    let next_run = last_run.saturating_add(interval.as_secs());
    if now >= next_run {
        Ok(AutomaticCacheCleanupDecision::Due)
    } else {
        Ok(AutomaticCacheCleanupDecision::Deferred { next_run })
    }
}

//! Experimental, development-only transactional TUF storage port.
//!
//! This is not a package security provider. A host must implement the required
//! admission interface with profile raw-key quorum and durable evidence/marker
//! checks before using it for authority. No accepted-time or grant mutation is
//! exposed. Storage implementations must atomically compare predecessor revisions,
//! persist exact evidence, and preserve provisioning and root continuity.
use async_trait::async_trait;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};
mod session;
pub(crate) use session::Session;

/// A storage revision, distinct from a signed metadata version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision(pub u64);

/// A retained metadata role; these are logical identifiers, never file paths.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MetadataRole {
    /// Timestamp rollback metadata.
    Timestamp,
    /// Snapshot rollback metadata.
    Snapshot,
    /// Top-level targets evidence.
    Targets,
    /// Delegated targets evidence, with the upstream logical role name.
    Delegated(String),
}

/// One consistent read of an explicitly provisioned protected store.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Current CAS revision.
    pub revision: Revision,
    /// Original out-of-band provisioned root, retained independently.
    pub provisioned_root: Vec<u8>,
    /// Exact currently trusted root envelope.
    pub current_root: Vec<u8>,
    /// Exact root at the beginning of an unfinished root-update cycle.
    pub reset_baseline: Option<Vec<u8>>,
    /// Existing rollback metadata and retained target evidence.
    pub metadata: BTreeMap<MetadataRole, Vec<u8>>,
    /// Last successful fresh package authorization, read-only in this port.
    pub accepted_time: Option<Timestamp>,
}

/// The upstream root-cycle outcome for timestamp/snapshot rollback state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reset {
    /// Retain both existing role floors.
    Preserve,
    /// Reset timestamp and snapshot together after the final root check.
    TimestampAndSnapshot,
}

/// A candidate transition produced after the corresponding upstream TUF checks.
/// It still requires host profile admission; it is not a package authorization.
#[derive(Debug, Clone)]
pub enum Transition {
    /// Atomically persist a successor root, continuity, and original reset baseline.
    AdvanceRoot {
        /// Exact fetched successor root bytes.
        root: Vec<u8>,
        /// Original root-cycle baseline, retained across restart.
        baseline: Vec<u8>,
    },
    /// Atomically apply the reset and finish the root cycle.
    FinishRootCycle {
        /// Whether both floors are reset.
        reset: Reset,
    },
    /// Retain exact bytes at the upstream role-acceptance point.
    Retain {
        /// Logical role.
        role: MetadataRole,
        /// Exact fetched signed envelope.
        bytes: Vec<u8>,
    },
}

/// Protected-state failures are errors, never missing optional metadata.
#[derive(Debug)]
pub enum Error {
    /// The expected predecessor revision no longer matches.
    Conflict,
    /// Established protected state is malformed or inconsistent.
    Corrupt(&'static str),
    /// The required host admission rejected the operation or transition.
    Admission,
    /// Fixed trusted time is earlier than the retained successful authorization.
    TimeRollback {
        /// Requested fixed time.
        fixed_time: Timestamp,
        /// Retained successful authorization time.
        accepted_time: Timestamp,
    },
    /// Backend I/O or transaction failure.
    Backend(Box<dyn std::error::Error + Send + Sync>),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict => f.write_str("protected-state predecessor changed"),
            Self::Corrupt(message) => write!(f, "corrupt protected state: {message}"),
            Self::Admission => f.write_str("required profile admission rejected"),
            Self::TimeRollback {
                fixed_time,
                accepted_time,
            } => write!(
                f,
                "trusted time {fixed_time} precedes accepted authorization {accepted_time}"
            ),
            Self::Backend(error) => write!(f, "protected-state backend: {error}"),
        }
    }
}
impl std::error::Error for Error {}
/// Result of a protected-state operation.
pub type Result<T> = std::result::Result<T, Error>;

/// Transactional host storage. Open existing state without implicit provisioning.
#[async_trait]
pub trait Storage: fmt::Debug + Send + Sync {
    /// Read all fields from one consistent transaction; report corruption as error.
    /// The host must validate retained provisioning, root continuity, reset context
    /// and required record presence. Session checks of encoding and self-signatures
    /// do not establish consistency of an arbitrary protected snapshot.
    async fn snapshot(&self) -> Result<Snapshot>;
    /// Commit only if `expected` still matches; return the committed successor revision.
    /// Persist the transition atomically before returning. Root advancement must also
    /// retain continuity. Never alter provisioning or accepted authorization time.
    async fn commit(&self, expected: Revision, transition: &Transition) -> Result<Revision>;
}

/// Mandatory host admission. There is intentionally no default implementation.
/// Tests use an explicitly test-only guard; real package admission is pending.
#[async_trait]
pub trait Admission: fmt::Debug + Send + Sync {
    /// Before current metadata authentication, require serialized predecessor access
    /// and durable candidate evidence/marker admission at the fixed host time.
    async fn begin(&self, predecessor: &Snapshot, fixed_time: Timestamp) -> Result<()>;
    /// Before any authority transition, require applicable profile checks, including
    /// distinct-raw-key root thresholds and binding to the admitted candidate evidence.
    async fn transition(&self, predecessor: &Snapshot, transition: &Transition) -> Result<()>;
}

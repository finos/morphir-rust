//! Errors reported while selecting or acquiring distribution artifacts.

use std::path::PathBuf;

/// A failure to parse, resolve, verify, or persist an extension artifact.
#[derive(Debug, thiserror::Error)]
pub enum DistributionError {
    /// A domain value did not satisfy its portable representation.
    #[error("invalid {kind} {value:?}: {reason}")]
    InvalidValue {
        /// The kind of value that failed validation.
        kind: &'static str,
        /// The rejected input.
        value: String,
        /// A concise validation rule.
        reason: &'static str,
    },
    /// One JSONL record could not be decoded.
    #[error("invalid extension index record on line {line}: {source}")]
    InvalidRecord {
        /// One-based JSONL line number.
        line: usize,
        /// The JSON decoding failure.
        source: serde_json::Error,
    },
    /// The history used an unsupported schema revision.
    #[error("unsupported extension index schema version {version} on line {line}")]
    UnsupportedSchema {
        /// One-based JSONL line number.
        line: usize,
        /// Unsupported schema version.
        version: u32,
    },
    /// A JSONL history did not contain any releases.
    #[error("extension history is empty")]
    EmptyHistory,
    /// Records for more than one extension appeared in one history.
    #[error("extension history mixes identities {expected:?} and {actual:?} on line {line}")]
    MixedIdentity {
        /// Identity established by the first record.
        expected: String,
        /// Identity found later in the history.
        actual: String,
        /// One-based JSONL line number.
        line: usize,
    },
    /// Two records declared the same exact version.
    #[error("duplicate version {version} in extension history")]
    DuplicateVersion {
        /// Duplicated semantic version.
        version: semver::Version,
    },
    /// Two versions differ only by build metadata, which has no SemVer precedence.
    #[error("versions {first} and {second} have equal semantic precedence")]
    DuplicatePrecedence {
        /// First version from the history.
        first: semver::Version,
        /// Later version with equal precedence.
        second: semver::Version,
    },
    /// No release and platform artifact satisfied the requested selection.
    #[error("no artifact matches {selection} for platform {platform}")]
    NoMatchingArtifact {
        /// Requested channel or exact version.
        selection: String,
        /// Requested operating-system and architecture pair.
        platform: String,
    },
    /// A release declared multiple artifacts for one platform.
    #[error("release {version} contains more than one artifact for platform {platform}")]
    AmbiguousPlatform {
        /// Exact release version.
        version: semver::Version,
        /// Requested operating-system and architecture pair.
        platform: String,
    },
    /// A canonical local-index path escaped its controlled root.
    #[error("path {path} escapes local index root {root}")]
    IndexPathEscape {
        /// Canonical path outside the controlled root.
        path: PathBuf,
        /// Canonical controlled index root.
        root: PathBuf,
    },
    /// Artifact bytes did not match the declared digest.
    #[error("digest mismatch for {path}: expected {expected}, got {actual}")]
    DigestMismatch {
        /// Source or installed artifact path.
        path: PathBuf,
        /// Digest declared by the exact lock or index.
        expected: crate::Sha256Digest,
        /// Digest computed from the file bytes.
        actual: crate::Sha256Digest,
    },
    /// Existing content has different executable semantics than the manifest.
    #[error(
        "executable mode mismatch for {path}: expected executable={expected}, got executable={actual}"
    )]
    ExecutableModeMismatch {
        /// Materialized artifact path.
        path: PathBuf,
        /// Executable state declared by the selected artifact.
        expected: bool,
        /// Executable state present on disk.
        actual: bool,
    },
    /// Durable JSON state could not be decoded.
    #[error("invalid distribution state in {path}: {source}")]
    InvalidState {
        /// State file being decoded.
        path: PathBuf,
        /// JSON decoding failure.
        source: serde_json::Error,
    },
    /// Durable JSON state could not be encoded.
    #[error("failed to encode distribution state: {0}")]
    StateEncoding(serde_json::Error),
    /// Durable state uses an unsupported schema revision.
    #[error("unsupported {kind} schema version {version}")]
    UnsupportedStateSchema {
        /// Kind of durable state.
        kind: &'static str,
        /// Unsupported schema version.
        version: u32,
    },
    /// No installed catalog entry exists for the requested identity.
    #[error("extension {id} is not installed")]
    NotInstalled {
        /// Requested extension identity.
        id: crate::ExtensionId,
    },
    /// Catalog and lock records disagree about exact installed content.
    #[error("installed catalog and lock disagree for extension {id}")]
    StateMismatch {
        /// Extension whose durable records disagree.
        id: crate::ExtensionId,
    },
    /// A state transaction failed and its previous files could not be restored.
    #[error("distribution state update failed ({original}); rollback also failed ({rollback})")]
    StateRollback {
        /// Original transaction failure.
        original: Box<DistributionError>,
        /// Failure encountered while restoring the previous state.
        rollback: Box<DistributionError>,
    },
    /// A catalog store path escaped the Morphir home directory.
    #[error("installed path {path} escapes Morphir home {root}")]
    InstalledPathEscape {
        /// Canonical installed artifact path.
        path: PathBuf,
        /// Canonical Morphir home root.
        root: PathBuf,
    },
    /// Filesystem access failed.
    #[error("failed to access {path}: {source}")]
    Io {
        /// Path being accessed.
        path: PathBuf,
        /// Underlying I/O failure.
        source: std::io::Error,
    },
}

/// Result type used by distribution operations.
pub type Result<T> = std::result::Result<T, DistributionError>;

pub(crate) fn invalid_value(
    kind: &'static str,
    value: impl Into<String>,
    reason: &'static str,
) -> DistributionError {
    DistributionError::InvalidValue {
        kind,
        value: value.into(),
        reason,
    }
}

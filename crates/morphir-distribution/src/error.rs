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

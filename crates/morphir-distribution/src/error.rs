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
    /// Tool releases for more than one identity were supplied to one resolution.
    #[error("tool releases mix identities {expected} and {actual}")]
    MixedToolIdentity {
        /// Identity established by the first release.
        expected: crate::ToolId,
        /// Different identity found later in the input.
        actual: crate::ToolId,
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
    /// Matching releases do not support any MEP version understood by the host.
    #[error("no release matching {selection} supports host MEP versions {supported}")]
    NoCompatibleMepVersion {
        /// Requested channel or exact version.
        selection: String,
        /// Host-supported MEP versions.
        supported: String,
    },
    /// Matching tool releases do not support the invoking Morphir CLI.
    #[error("no release matching {selection} supports Morphir CLI {cli_version}")]
    NoCompatibleCliVersion {
        /// Requested channel or exact version.
        selection: String,
        /// Invoking Morphir CLI version.
        cli_version: semver::Version,
    },
    /// An exact tool release has been revoked by its publisher.
    #[error("tool {tool} version {version} is revoked")]
    RevokedToolRelease {
        /// Revoked tool identity.
        tool: crate::ToolId,
        /// Revoked exact semantic version.
        version: semver::Version,
    },
    /// TUF rejected repository metadata or target bytes.
    #[error("tool repository verification failed: {source}")]
    ToolRepository {
        /// Underlying TUF client failure.
        #[source]
        source: Box<tough::error::Error>,
    },
    /// Authenticated Morphir-specific target metadata is malformed or inconsistent.
    #[error("invalid authenticated tool metadata for {target}: {reason}")]
    InvalidToolMetadata {
        /// Authenticated target path.
        target: String,
        /// Concise consistency or decoding failure.
        reason: String,
    },
    /// Authenticated repository metadata does not list a required target.
    #[error("authenticated tool target {target} is missing")]
    MissingToolTarget {
        /// Required target path.
        target: String,
    },
    /// Installed tool bytes do not have the authenticated target length.
    #[error("length mismatch for {path}: expected {expected} bytes, got {actual}")]
    ToolLengthMismatch {
        /// Installed tool path.
        path: PathBuf,
        /// Length authenticated by TUF targets metadata.
        expected: u64,
        /// Observed file length.
        actual: u64,
    },
    /// The selected packaging format is not supported by this installation path.
    #[error("tool archive format {format} is not supported by this installer")]
    UnsupportedToolArchive {
        /// Unsupported archive format.
        format: String,
    },
    /// A raw tool's launch path does not name the downloaded target file.
    #[error("raw tool launch path {entry_point} does not match target file {target}")]
    ToolEntryPointMismatch {
        /// Downloaded target filename.
        target: String,
        /// Declared launch entry point.
        entry_point: String,
    },
    /// An archive entry violates portable and contained extraction rules.
    #[error("unsafe tool archive entry {entry:?}: {reason}")]
    UnsafeToolArchive {
        /// Entry name as supplied by the archive.
        entry: String,
        /// Rejected archive property.
        reason: String,
    },
    /// Installed package manifest is incomplete or internally inconsistent.
    #[error("invalid installed tool package manifest: {reason}")]
    InvalidToolManifest {
        /// Violated manifest invariant.
        reason: String,
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
    /// An installed extension artifact exceeds the activation memory budget.
    #[error("artifact {path:?} is {actual} bytes, exceeding the {limit}-byte activation limit")]
    ArtifactTooLarge {
        /// Installed artifact path.
        path: PathBuf,
        /// Observed byte length.
        actual: u64,
        /// Maximum accepted byte length.
        limit: u64,
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
    /// A repository configuration already uses the requested name.
    #[error("extension repository {name} is already configured")]
    RepositoryAlreadyExists {
        /// Conflicting repository name.
        name: crate::RepositoryName,
    },
    /// No repository configuration uses the requested name.
    #[error("extension repository {name} is not configured")]
    RepositoryNotFound {
        /// Missing repository name.
        name: crate::RepositoryName,
    },
    /// A disabled repository was selected for extension resolution.
    #[error("extension repository {name} is disabled")]
    RepositoryDisabled {
        /// Disabled repository name.
        name: crate::RepositoryName,
    },
    /// A local repository history filename disagrees with its declared identity.
    #[error(
        "extension repository history {path} declares {actual}, but its filename identifies {expected}"
    )]
    RepositoryHistoryIdentity {
        /// History file being validated.
        path: PathBuf,
        /// Identity derived from the history filename.
        expected: crate::ExtensionId,
        /// Identity declared by the history records.
        actual: crate::ExtensionId,
    },
    /// A staged extension release bundle violated its format or path contract.
    #[error("invalid extension release bundle at {path}: {reason}")]
    InvalidReleaseBundle {
        /// Bundle path or entry that failed validation.
        path: PathBuf,
        /// Concise validation failure.
        reason: String,
    },
    /// A repository already contains a different release at the same SemVer precedence.
    #[error("extension repository already contains a different {id} release at version {version}")]
    RepositoryReleaseConflict {
        /// Conflicting extension identity.
        id: crate::ExtensionId,
        /// Version requested for conflicting publication.
        version: semver::Version,
    },
    /// No installed catalog entry exists for the requested identity.
    #[error("extension {id} is not installed")]
    NotInstalled {
        /// Requested extension identity.
        id: crate::ExtensionId,
    },
    /// No installed tool catalog entry exists for the requested identity.
    #[error("tool {id} is not installed")]
    ToolNotInstalled {
        /// Requested tool identity.
        id: crate::ToolId,
    },
    /// A create-only installation found an existing catalog entry.
    #[error("tool {id} is already installed")]
    ToolAlreadyInstalled {
        /// Requested tool identity.
        id: crate::ToolId,
    },
    /// An installed tool has no retained release eligible for rollback.
    #[error("tool {id} has no retained rollback release")]
    NoToolRollback {
        /// Requested tool identity.
        id: crate::ToolId,
    },
    /// Authenticated repair bytes do not describe the installed exact release.
    #[error("repair candidate for tool {id} does not match installed release {version}: {reason}")]
    ToolRepairMismatch {
        /// Installed tool identity.
        id: crate::ToolId,
        /// Installed exact semantic version.
        version: semver::Version,
        /// Metadata or package field that did not match.
        reason: &'static str,
    },
    /// Catalog and lock records disagree about exact installed content.
    #[error("installed catalog and lock disagree for extension {id}")]
    StateMismatch {
        /// Extension whose durable records disagree.
        id: crate::ExtensionId,
    },
    /// One installed record violates its runtime-specific state invariants.
    #[error("invalid installed state for extension {id}: {reason}")]
    InvalidInstalledState {
        /// Extension whose persisted runtime state is malformed.
        id: crate::ExtensionId,
        /// Runtime invariant that the record violates.
        reason: &'static str,
    },
    /// Tool catalog and exact lock disagree about active content.
    #[error("installed catalog and lock disagree for tool {id}")]
    ToolStateMismatch {
        /// Tool whose durable records disagree.
        id: crate::ToolId,
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

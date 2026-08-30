use serde::{Deserialize, Serialize};

use crate::RelativePath;

/// Stable diagnostic code for a missing workspace configuration.
pub const WORKSPACE_CONFIG_MISSING: &str = "workspace.config.missing";

/// Stable diagnostic code for ambiguous workspace configurations.
pub const WORKSPACE_CONFIG_AMBIGUOUS: &str = "workspace.config.ambiguous";

/// Stable diagnostic code for an invalid workspace configuration.
pub const WORKSPACE_CONFIG_INVALID: &str = "workspace.config.invalid";

/// Stable diagnostic code for an invalid workspace member.
pub const WORKSPACE_MEMBER_INVALID: &str = "workspace.member.invalid";

/// Stable diagnostic code for duplicate workspace member names.
pub const WORKSPACE_MEMBER_DUPLICATE_NAME: &str = "workspace.member.duplicate-name";

/// Stable diagnostic code for a path that escapes its named mount.
pub const WORKSPACE_PATH_NOT_CONFINED: &str = "workspace.path.not-confined";

/// Stable diagnostic code for an unsupported discovery protocol version.
pub const WORKSPACE_PROTOCOL_UNSUPPORTED: &str = "workspace.protocol.unsupported";

/// The severity of a workspace discovery diagnostic.
///
/// When severity participates in a stable diagnostic ordering key, producers
/// order variants as `Info`, `Warning`, then `Error`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DiagnosticSeverity {
    /// Context that does not indicate a problem.
    Info,
    /// A recoverable problem that may require attention.
    Warning,
    /// A problem that prevented some requested discovery work.
    Error,
}

/// A provider-neutral diagnostic emitted during workspace discovery.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDiagnostic {
    /// The diagnostic severity.
    pub severity: DiagnosticSeverity,
    /// A stable, machine-readable diagnostic code.
    pub code: String,
    /// A human-readable explanation.
    pub message: String,
    /// The path most directly associated with the diagnostic, when known.
    pub path: Option<RelativePath>,
    /// The project path associated with the diagnostic, when known.
    pub project_path: Option<RelativePath>,
}

/// A fatal, provider-neutral workspace discovery failure.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryFailure {
    /// A stable, machine-readable failure code.
    pub code: String,
    /// A human-readable explanation.
    pub message: String,
    /// The path associated with the failure, when known.
    pub path: Option<RelativePath>,
}

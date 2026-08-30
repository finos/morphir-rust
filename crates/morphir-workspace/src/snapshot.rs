use serde::{Deserialize, Serialize};

use crate::{DiscoveryFailure, RelativePath, WorkspaceDiagnostic};

/// The state of a discovered workspace.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceState {
    /// The workspace was opened successfully.
    Open,
    /// The workspace was opened with workspace-level errors.
    Error,
}

/// The state of a discovered project.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectState {
    /// The project has been discovered but not loaded.
    Unloaded,
    /// The project could not be discovered completely.
    Error,
}

/// A deterministic, provider-neutral snapshot of a workspace.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshot {
    /// The protocol version used to produce this snapshot.
    pub protocol_version: u32,
    /// The workspace configuration path relative to the development root.
    pub config_anchor: RelativePath,
    /// The configured workspace name, when present.
    pub name: Option<String>,
    /// The overall workspace state.
    pub state: WorkspaceState,
    /// Discovered projects, sorted by canonical relative path and then name.
    pub projects: Vec<ProjectSnapshot>,
    /// Workspace diagnostics, sorted by project path, path, code, severity,
    /// and message.
    pub diagnostics: Vec<WorkspaceDiagnostic>,
}

/// A deterministic, provider-neutral snapshot of a project.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSnapshot {
    /// The project name.
    pub name: String,
    /// The project version, when configured.
    pub version: Option<String>,
    /// The project path relative to the development root.
    pub relative_path: RelativePath,
    /// The project configuration path, when one exists.
    pub config_anchor: Option<RelativePath>,
    /// The source directory relative to this project's [`Self::relative_path`].
    pub source_directory: RelativePath,
    /// The project state.
    pub state: ProjectState,
    /// Project diagnostics, sorted by project path, path, code, severity, and
    /// message.
    pub diagnostics: Vec<WorkspaceDiagnostic>,
}

/// The provider-neutral result of workspace discovery.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum DiscoveryResponse {
    /// Discovery completed and produced a snapshot.
    Success {
        /// The discovered workspace snapshot.
        snapshot: WorkspaceSnapshot,
    },
    /// Discovery failed before a snapshot could be produced.
    Failure {
        /// The fatal discovery failure.
        error: DiscoveryFailure,
    },
}

impl DiscoveryResponse {
    /// Converts the wire response into a conventional Rust result.
    pub fn into_result(self) -> Result<WorkspaceSnapshot, DiscoveryFailure> {
        match self {
            Self::Success { snapshot } => Ok(snapshot),
            Self::Failure { error } => Err(error),
        }
    }
}

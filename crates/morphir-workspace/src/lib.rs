//! Portable protocol types for Morphir workspace discovery.

pub mod config;
mod diagnostic;
mod discovery;
mod file_tree;
mod path;
mod snapshot;

pub use diagnostic::{
    DiagnosticSeverity, DiscoveryFailure, WORKSPACE_CONFIG_AMBIGUOUS, WORKSPACE_CONFIG_INVALID,
    WORKSPACE_CONFIG_MISSING, WORKSPACE_MEMBER_DUPLICATE_NAME, WORKSPACE_MEMBER_INVALID,
    WORKSPACE_PATH_NOT_CONFINED, WORKSPACE_PROTOCOL_UNSUPPORTED, WorkspaceDiagnostic,
};
pub use discovery::{discover, discover_with_details};
pub use file_tree::{DiscoveryRequest, FileEntry, FileTree, WORKSPACE_DISCOVERY_PROTOCOL};
pub use path::{RelativePath, RelativePathError};
pub use snapshot::{
    DiscoveryResponse, ProjectSnapshot, ProjectState, WorkspaceDiscoveryDetails, WorkspaceSnapshot,
    WorkspaceState,
};

//! Portable protocol types for Morphir workspace discovery.

pub mod config;
mod diagnostic;
mod discovery;
mod file_tree;
mod path;
mod snapshot;

pub use diagnostic::{
    DiagnosticSeverity, DiscoveryFailure, WORKSPACE_CONFIG_AMBIGUOUS, WORKSPACE_CONFIG_INVALID,
    WORKSPACE_CONFIG_MISSING, WORKSPACE_LANGUAGE_ID_EMPTY, WORKSPACE_MEMBER_DUPLICATE_NAME,
    WORKSPACE_MEMBER_INVALID, WORKSPACE_PATH_NOT_CONFINED, WORKSPACE_PROJECT_NAME_EMPTY,
    WORKSPACE_PROTOCOL_UNSUPPORTED, WORKSPACE_PURPOSE_UNSUPPORTED, WORKSPACE_SELECTION_DUPLICATE,
    WORKSPACE_SELECTION_EMPTY, WORKSPACE_SELECTION_INVALID, WORKSPACE_SELECTION_NAME_REQUIRED,
    WORKSPACE_SELECTION_OUTSIDE_ROOT, WORKSPACE_SYMLINK_UNSUPPORTED, WorkspaceDiagnostic,
};
pub use discovery::{discover, discover_with_details};
pub use file_tree::{
    DiscoveryPurpose, DiscoveryRequest, FileEntry, FileTree, ProjectSource, SourceSelection,
    WORKSPACE_DISCOVERY_PROTOCOL,
};
pub use path::{RelativePath, RelativePathError};
pub use snapshot::{
    DiscoveryResponse, ProjectOrigin, ProjectSnapshot, ProjectState, WorkspaceDiscoveryDetails,
    WorkspaceSnapshot, WorkspaceState,
};

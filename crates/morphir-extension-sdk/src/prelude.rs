//! Prelude module for convenient imports
//!
//! ```rust
//! use morphir_extension_sdk::prelude::*;
//! ```

// Re-export all core types
pub use crate::types::{
    Artifact, CompileRequest, CompileResult, Diagnostic, DiagnosticSeverity, ExtensionCapabilities,
    ExtensionInfo, ExtensionType, GenerateRequest, GenerateResult, RelatedInformation,
    ResourceLimits, SourceFile, SourceLocation, TransformRequest, TransformResult,
    ValidateRequest, ValidateResult, WorkspaceInfo,
};

// Re-export traits
pub use crate::traits::{Backend, Extension, Frontend, Transform, Validator};

// Re-export error types
pub use crate::error::{ExtensionError, Result};

// Re-export protocol types
pub use crate::protocol::{ExtensionRequest, ExtensionResponse, RpcError};

// Re-export macros
pub use crate::{export_extension, host_debug, host_error, host_info, host_warn};

// Re-export host functions
pub use crate::host::{cache_ir, get_cached_ir, get_config, get_var, get_workspace_info, set_var};

//! Prelude module for convenient imports
//!
//! ```rust
//! use morphir_extension_sdk::prelude::*;
//! ```

// Re-export all core types
pub use crate::types::{
    Artifact, BackendCapability, BaselineModule, CompileBaseline, CompileDependency,
    CompileOptions, CompilePackage, CompileRequest, CompileResult, Diagnostic, DiagnosticSeverity,
    ExtensionCapabilities, ExtensionInfo, ExtensionType, FrontendCapability, GenerateRequest,
    GenerateResult, LanguageCapability, ModuleResult, ModuleStatus, RelatedInformation,
    ResourceLimits, SourceDocument, SourceLocation, SourcePosition, SourceRange, SourceSet,
    TransformRequest, TransformResult, ValidateRequest, ValidateResult, WorkspaceCapability,
    WorkspaceInfo,
};

// Re-export traits
pub use crate::traits::{Backend, Extension, Frontend, Transform, Validator, Workspace};

// Re-export error types
pub use crate::error::{ExtensionError, Result};

// Re-export native extension adapters.
pub use crate::{NativeBackend, NativeExtension, NativeFrontend, NativeProtocol, NativeWorkspace};

// Re-export protocol types
pub use crate::protocol::{ExtensionRequest, ExtensionResponse, RpcError};

// Authoring macros remain available to native tests, but emit guest code only for WASM.
pub use crate::{export_extension, host_debug, host_error, host_info, host_warn};

#[cfg(target_arch = "wasm32")]
pub use crate::host::{cache_ir, get_cached_ir, get_config, get_var, get_workspace_info, set_var};

#[cfg(test)]
mod tests {
    use super::*;

    /// Every native adapter is reachable from the prelude. A role whose endpoint
    /// trait is missing here cannot be named in a trait-object field or a generic
    /// bound by a consumer that imports only the prelude, even though every other
    /// adapter can.
    #[test]
    fn prelude_exports_every_native_adapter() {
        let _: Option<&dyn NativeFrontend> = None;
        let _: Option<&dyn NativeBackend> = None;
        let _: Option<&dyn NativeWorkspace> = None;
        let _: Option<&dyn NativeProtocol> = None;
        let _: Option<NativeExtension> = None;
    }

    #[test]
    fn prelude_exports_the_frontend_and_backend_contracts() {
        let _: Option<BackendCapability> = None;
        let _: Option<CompileDependency> = None;
        let _: Option<CompileOptions> = None;
        let _: Option<CompilePackage> = None;
        let _: Option<FrontendCapability> = None;
        let _: Option<LanguageCapability> = None;
        let _: Option<SourceDocument> = None;
        let _: Option<SourcePosition> = None;
        let _: Option<SourceRange> = None;
        let _: Option<SourceSet> = None;
    }
}

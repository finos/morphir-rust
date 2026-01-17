//! Extension traits defining the contract between extensions and the host
//!
//! Extensions implement these traits to provide functionality.

use crate::error::Result;
use crate::types::*;

/// Base trait all extensions must implement
pub trait Extension {
    /// Return extension metadata
    fn info() -> ExtensionInfo;

    /// Return extension capabilities for runtime negotiation
    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities::default()
    }
}

/// Frontend extension: parses source code to Morphir IR
///
/// Frontends take source files in a specific language and produce
/// Morphir IR representation.
pub trait Frontend: Extension {
    /// Compile source files to IR
    fn compile(&self, request: CompileRequest) -> Result<CompileResult>;

    /// Languages this frontend supports (e.g., ["gleam", "elm"])
    fn supported_languages() -> Vec<String>;

    /// File extensions this frontend handles (e.g., [".gleam", ".elm"])
    fn file_extensions() -> Vec<String>;
}

/// Backend extension: generates code from Morphir IR
///
/// Backends take Morphir IR and produce code in a target language.
pub trait Backend: Extension {
    /// Generate code from IR
    fn generate(&self, request: GenerateRequest) -> Result<GenerateResult>;

    /// Target languages this backend produces (e.g., ["wasm", "gleam"])
    fn target_languages() -> Vec<String>;
}

/// Validator extension: analyzes IR and produces diagnostics
///
/// Validators examine IR for correctness, style issues, or other concerns.
pub trait Validator: Extension {
    /// Validate IR and return diagnostics
    fn validate(&self, request: ValidateRequest) -> Result<ValidateResult>;

    /// Names of validation rules this validator provides
    fn validation_rules() -> Vec<String> {
        Vec::new()
    }
}

/// Transform extension: transforms IR to IR
///
/// Transforms modify or optimize the IR representation.
pub trait Transform: Extension {
    /// Transform IR
    fn transform(&self, request: TransformRequest) -> Result<TransformResult>;

    /// Names of transformations this extension provides
    fn transformation_names() -> Vec<String> {
        Vec::new()
    }
}

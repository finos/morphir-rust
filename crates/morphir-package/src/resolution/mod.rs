//! Deterministic resolution for the experimental `flat-library` profile.
//!
//! Resolution consumes a complete, immutable metadata snapshot. It never
//! acquires package contents or treats a selected graph as proof of integrity.

mod diagnostics;
mod execution;
mod model;
mod order;
mod parse;
mod replay;
mod search;
mod update;
mod validate;
mod wire;

pub use model::{
    Binding, Catalog, ChangedPin, IrPackageName, LockedGraph, LockedNode, MissingItem, PackagePath,
    ReleaseId, ReleaseRecord, RequiredCapability, Requirement, ResolutionDiagnostic,
    ResolutionDigest, ResolutionExecutionError, ResolutionResult, ResolutionValueError,
    StableVersion, UpdateTarget, VersionRange, Violation, ViolationRule, Witness, WitnessBinding,
    WitnessNode,
};

/// The experimental package contract implemented by this module.
pub const CONTRACT_VERSION: &str = "0.1.0-draft.2";

/// Resolve one draft-2 Library metadata document.
///
/// Domain rejections are returned as [`ResolutionResult::Rejected`]. Failures
/// to execute the bounded operation are returned separately.
///
/// ```
/// use morphir_package::resolution::{resolve_library, ResolutionResult};
///
/// let input = r#"{
///   "formatVersion":"0.1.0-draft.2",
///   "capability":"flat-library",
///   "mode":"initial",
///   "root":{
///     "release":{"packagePath":"example.com/app/root","version":"1.0.0"},
///     "irPackageName":"example/app",
///     "manifestDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000",
///     "contentDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000",
///     "dependencies":[]
///   },
///   "catalogs":[]
/// }"#;
/// assert!(matches!(resolve_library(input)?, ResolutionResult::Resolved(_)));
/// # Ok::<(), morphir_package::resolution::ResolutionExecutionError>(())
/// ```
pub fn resolve_library(input: &str) -> Result<ResolutionResult, ResolutionExecutionError> {
    parse::resolve(input)
}

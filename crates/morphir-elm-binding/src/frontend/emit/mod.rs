//! Lowering the resolved model to a Morphir IR distribution.
//!
//! Each supported IR version has its own emitter, and both lower from the same
//! version-neutral [`crate::resolved`] model: a v4 document is written natively
//! rather than migrated from a classic one.
//!
//! An emitter writes a package in two steps so that an incremental build can keep
//! the modules it did not have to re-resolve. [`Emitter::emit_module`] writes one
//! module's access-controlled definition, which the caller stores per module;
//! [`Emitter::emit_distribution`] assembles a whole distribution out of those
//! stored values, reading them back into the core model rather than re-resolving.

pub mod classic;
pub mod v4;

use serde_json::Value;

use crate::resolved::{Access, ResolvedModule};

/// Everything a distribution needs beyond its modules' definitions.
pub struct PackageInput<'a> {
    /// The package path, one PascalCase segment per element.
    pub package: &'a [String],
    /// The package's resolved modules, in the order they are written.
    pub modules: &'a [ResolvedModule],
    /// Dependency specifications as `(package path, classic package specification
    /// JSON)`.
    ///
    /// Only the classic emitter uses these: a classic distribution carries its
    /// dependencies' specifications inline, and the JSON is passed through as
    /// given. A v4 `Library` keys its dependencies by canonical package name, and
    /// nothing supplies those specifications yet, so the v4 emitter writes none.
    pub dependencies: &'a [(Vec<String>, Value)],
}

/// One module's emitted definition, as [`Emitter::emit_distribution`] takes it:
/// the module path, its access, and the JSON [`Emitter::emit_module`] wrote.
pub type ModuleIr = (Vec<String>, Access, Value);

/// Writes a Morphir IR document for one IR version.
pub trait Emitter {
    /// The module's access-controlled definition, for the per-module baseline.
    fn emit_module(&self, module: &ResolvedModule) -> Value;

    /// A whole distribution, assembled from per-module values.
    ///
    /// Each value must be one this emitter's [`Emitter::emit_module`] wrote;
    /// anything else is a caller error and panics rather than writing a document
    /// no reader accepts.
    fn emit_distribution(&self, input: &PackageInput, module_irs: &[ModuleIr]) -> Value;

    /// The IR version this emitter writes, as the extension spells it.
    fn format_version(&self) -> &'static str;
}

/// The emitter for an IR version, or nothing when the version is not one of ours.
pub fn emitter_for(ir_version: &str) -> Option<Box<dyn Emitter>> {
    match ir_version {
        "3" => Some(Box::new(classic::ClassicEmitter)),
        "4" => Some(Box::new(v4::V4Emitter)),
        _ => None,
    }
}

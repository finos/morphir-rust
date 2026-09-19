//! Lowering the resolved model to a Morphir IR distribution.
//!
//! Each supported IR version has its own emitter, and both lower from the same
//! version-neutral [`crate::resolved`] model: a v4 document is written natively
//! rather than migrated from a classic one. The one step they share is name
//! decomposition, which lives in [`crate::names`].
//!
//! An emitter writes a package in two steps so that an incremental build can keep
//! the modules it did not have to re-resolve. [`Emitter::emit_module`] writes one
//! module's access-controlled definition, which the caller stores per module;
//! [`Emitter::emit_distribution`] assembles a whole distribution out of those
//! stored values, reading them back into the core model rather than re-resolving.
//! Both are fallible: a stored value may be stale, hand-edited or written by the
//! other version's emitter, and two names may collide once they are canonical, so
//! the emitters report rather than panic or overwrite.

pub mod classic;
pub mod v4;

use serde::Serialize;
use serde_json::Value;

use crate::names::words;
use crate::resolved::{Access, ResolvedModule};

/// The order the modules, types and constructors of a document are written in.
///
/// This changes the document, so it is part of the compile context
/// ([`crate::frontend::boundary::context_digest`]): a run must not assemble a
/// distribution out of modules emitted under the other order.
///
/// Record fields and constructor arguments are *not* affected. They are
/// positional in morphir-elm too — it keeps them in a list, not a `Dict` — so
/// source order is already the matching order for both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ordering {
    /// The order the source declares things in. This is the default: a reader
    /// comparing a document with the Elm it came from finds them in the same
    /// order, and a diff between two versions of a package shows the edit
    /// rather than a reshuffle.
    Source,
    /// The order morphir-elm writes.
    ///
    /// morphir-elm holds modules, types and constructors in Elm `Dict`s, so the
    /// order in its JSON is the `Dict`'s key order. A key is a `Path`
    /// (`List Name`) or a `Name` (`List String`), compared element by element
    /// with a shorter prefix first — which is exactly Rust's `Ord` on
    /// `Vec<Vec<String>>` and `Vec<String>`. So the sort is on the *words* a
    /// name splits into, never on a rendered spelling of it: `LocalDate` sorts
    /// as `["local", "date"]`, and `SDK` as `["s", "d", "k"]`.
    MorphirElm,
}

/// Resolves the `elmOrdering` extension option into an [`Ordering`].
///
/// - `None` selects [`Ordering::Source`].
/// - `"source"` and `"morphir-elm"` name the two orders.
pub fn ordering_from_option(value: Option<&Value>) -> Result<Ordering, String> {
    match value {
        None => Ok(Ordering::Source),
        Some(Value::String(order)) => match order.as_str() {
            "source" => Ok(Ordering::Source),
            "morphir-elm" => Ok(Ordering::MorphirElm),
            other => Err(format!(
                "unknown order `{other}`: expected `source` or `morphir-elm`"
            )),
        },
        Some(other) => Err(format!("expected a string, got {other}")),
    }
}

/// The key a `Dict` keyed by `Name` orders an identifier by: its words.
pub(crate) fn name_key(source: &str) -> Vec<String> {
    words(source)
}

/// The key a `Dict` keyed by `Path` orders a module path by: one word list per
/// segment.
pub(crate) fn path_key(segments: &[String]) -> Vec<Vec<String>> {
    segments.iter().map(|segment| words(segment)).collect()
}

/// `items`, in the order `ordering` asks for, given the sort key of each.
pub(crate) fn in_order<'a, T, K: Ord>(
    items: impl IntoIterator<Item = &'a T>,
    ordering: Ordering,
    key: impl Fn(&T) -> K,
) -> Vec<&'a T>
where
    T: 'a,
{
    let mut items: Vec<&T> = items.into_iter().collect();
    if ordering == Ordering::MorphirElm {
        items.sort_by_key(|item| key(item));
    }
    items
}

/// Everything a distribution needs beyond its modules' definitions.
pub struct PackageInput<'a> {
    /// The package path, one PascalCase segment per element.
    pub package: &'a [String],
    /// The package's resolved modules, in the order they are written.
    pub modules: &'a [ResolvedModule],
    /// Dependency specifications as `(package path, package specification JSON)`.
    ///
    /// The JSON is read by the emitter that is writing, in that emitter's own
    /// version: the classic emitter decodes each value as a classic
    /// `PackageSpecification` and lists them inline, and the v4 emitter decodes
    /// each as a v4 `PackageSpecification` and keys them by canonical package
    /// name. A caller therefore supplies the specifications of the version it
    /// asked for, and mixing the two is an emit error rather than a silent
    /// mis-write.
    pub dependencies: &'a [(Vec<String>, Value)],
}

/// One module's emitted definition, as [`Emitter::emit_distribution`] takes it:
/// the module path, its access, and the JSON [`Emitter::emit_module`] wrote.
pub type ModuleIr = (Vec<String>, Access, Value);

/// What an emitter reports when it cannot write the document asked of it.
pub type EmitError = String;

/// Writes a Morphir IR document for one IR version.
pub trait Emitter {
    /// The module's access-controlled definition, for the per-module baseline.
    ///
    /// Fails when two declarations in the module collide once their names are
    /// canonical, since one would otherwise silently replace the other.
    fn emit_module(&self, module: &ResolvedModule) -> Result<Value, EmitError>;

    /// A whole distribution, assembled from per-module values.
    ///
    /// Fails when a stored module value is not one this emitter's
    /// [`Emitter::emit_module`] wrote, or when two module paths collide once they
    /// are canonical.
    fn emit_distribution(
        &self,
        input: &PackageInput,
        module_irs: &[ModuleIr],
    ) -> Result<Value, EmitError>;

    /// The IR version this emitter writes, as the extension spells it.
    fn format_version(&self) -> &'static str;
}

/// The emitter for an IR version, or nothing when the version is not one of ours.
pub fn emitter_for(ir_version: &str, ordering: Ordering) -> Option<Box<dyn Emitter>> {
    match ir_version {
        "3" => Some(Box::new(classic::ClassicEmitter { ordering })),
        "4" => Some(Box::new(v4::V4Emitter { ordering })),
        _ => None,
    }
}

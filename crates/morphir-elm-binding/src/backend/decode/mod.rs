//! Reading a Morphir IR distribution back into the version-neutral resolved
//! model.
//!
//! This is the inverse of [`crate::frontend::emit`], and there is exactly one
//! decoder per IR version: the classic (v3) reader and the v4 reader both
//! produce [`crate::resolved::ResolvedModule`]s, so everything downstream — the
//! backend that writes Elm and the frontend that reads a baseline or a
//! dependency's interface — works on one model and cannot drift between
//! versions. Nothing here migrates a v3 document into a v4 one.
//!
//! Value definitions are counted, never decoded: this binding writes types.
//!
//! A construct a version can hold but Elm cannot write (a v4
//! `IncompleteTypeDefinition`) is not a decode failure. The declaration is
//! decoded as an opaque one — which is what
//! `PackageDefinition::to_specification` makes of it too, so a baseline
//! interface reads back the same — and named in [`Decoded::unsupported`], so
//! that a caller generating source can report it and leave it out while the
//! rest of the module is still written.

pub mod classic;
pub mod v4;

use morphir_core::format_version::{NormalizedFormatVersion, ScalarValue, SupportTable};
use morphir_extension_sdk::{Diagnostic, DiagnosticSeverity};
use serde_json::Value;

use crate::names;
use crate::resolved::{Interface, ResolvedModule};

/// IR this version's reader rejects.
pub const IR: &str = "ELM_IR";

/// A construct a Morphir document can hold that Elm has no form for.
pub const UNSUPPORTED: &str = "ELM_UNSUPPORTED";

/// A decoded distribution, ready to be raised into Elm.
pub struct DecodedPackage {
    /// The package path, one document-spelled segment per element.
    pub package: Vec<String>,
    /// The package's modules, in the order the document lists them.
    pub modules: Vec<ResolvedModule>,
    /// How many value definitions the document held. None of them is written.
    pub omitted_values: usize,
    /// One `ELM_UNSUPPORTED` diagnostic per construct with no Elm form.
    pub diagnostics: Vec<Diagnostic>,
}

/// A declaration a version can hold that Elm cannot write.
pub struct Unsupported {
    /// The IR node's own name, e.g. `IncompleteTypeDefinition`.
    pub construct: &'static str,
    /// The module the declaration is in, document-spelled.
    pub module: Vec<String>,
    /// The declaration's name, document-spelled.
    pub name: String,
}

/// What one version's reader produces: the package as the document holds it,
/// before declarations with no Elm form are dropped.
pub struct Decoded {
    pub package: Vec<String>,
    pub modules: Vec<ResolvedModule>,
    pub omitted_values: usize,
    pub unsupported: Vec<Unsupported>,
}

/// What one version's reader makes of a single module definition: the module,
/// how many values it held, and the declarations with no Elm form.
pub struct DecodedModule {
    pub module: ResolvedModule,
    pub omitted_values: usize,
    pub unsupported: Vec<Unsupported>,
}

/// Decodes a distribution, choosing the reader by its `formatVersion`.
///
/// Both the integer and the string spellings of a version are accepted, and a
/// patch release (`"3.1.0"`) reads as its major release, the way
/// [`morphir_core::format_version`] normalises one.
pub fn decode(ir: &Value) -> Result<DecodedPackage, Vec<Diagnostic>> {
    let major = major_version(ir)?;
    // The readers want the plain major version, so a document that spelled it
    // `"3.1.0"` is normalised before it is read.
    let mut ir = ir.clone();
    ir["formatVersion"] = major.into();

    let decoded = match major {
        3 => classic::decode(&ir),
        _ => v4::decode(&ir),
    }
    .map_err(|reason| vec![diagnostic(DiagnosticSeverity::Error, IR, reason)])?;

    let Decoded {
        package,
        mut modules,
        omitted_values,
        unsupported,
    } = decoded;

    let diagnostics = unsupported
        .iter()
        .map(|entry| {
            diagnostic(
                DiagnosticSeverity::Error,
                UNSUPPORTED,
                format!(
                    "`{}` has no Elm form: `{}:{}#{}` is left out of the generated source",
                    entry.construct,
                    package.join("."),
                    names::module_label(&entry.module),
                    entry.name
                ),
            )
        })
        .collect();

    // A declaration Elm cannot write is left out rather than written as
    // something it is not; everything else in its module is still generated.
    for module in &mut modules {
        module.types.retain(|declaration| {
            !unsupported
                .iter()
                .any(|entry| entry.module == module.name && entry.name == declaration.name)
        });
    }

    Ok(DecodedPackage {
        package,
        modules,
        omitted_values,
        diagnostics,
    })
}

/// The public interface one module definition describes.
///
/// The module is decoded through the very same reader a whole distribution goes
/// through, so a baseline entry's interface is the one resolution produced.
pub fn interface_of(
    ir_version: &str,
    name: &[String],
    module_ir: &Value,
) -> Result<Interface, String> {
    module_of(ir_version, name, module_ir).map(|decoded| decoded.module.interface())
}

/// One module definition, as the version's reader makes of it.
pub fn module_of(
    ir_version: &str,
    name: &[String],
    module_ir: &Value,
) -> Result<DecodedModule, String> {
    match ir_version {
        "3" => classic::module_definition(name, module_ir),
        "4" => v4::module_definition(name, module_ir),
        other => Err(format!("unsupported IR version `{other}`")),
    }
}

/// The major release a document's `formatVersion` names, when it is one this
/// backend generates from.
fn major_version(ir: &Value) -> Result<u32, Vec<Diagnostic>> {
    let reject = |reason: String| vec![diagnostic(DiagnosticSeverity::Error, IR, reason)];

    let stated = ir
        .get("formatVersion")
        .ok_or_else(|| reject("the document states no `formatVersion`".to_string()))?;
    let scalar = ScalarValue::from_json(stated)
        .map_err(|error| reject(format!("`formatVersion` is not a version: {error}")))?;
    let version = NormalizedFormatVersion::from_scalar(&scalar, &SupportTable::reference())
        .map_err(|error| reject(format!("`formatVersion` is not a known release: {error}")))?;

    let major = version.release.major();
    if !version.is_supported() || !matches!(major, 3 | 4) {
        return Err(reject(format!(
            "this backend generates Elm from Morphir IR 3 or 4; the document states {major}"
        )));
    }
    Ok(major)
}

fn diagnostic(severity: DiagnosticSeverity, code: &str, message: String) -> Diagnostic {
    Diagnostic {
        severity,
        code: Some(code.to_string()),
        message,
        location: None,
        related: Vec::new(),
    }
}

//! The Elm backend: a Morphir IR distribution in, Elm source out.
//!
//! Generation is three steps, and each is version-neutral after the first:
//!
//! 1. [`decode`] reads the document into the same [`crate::resolved`] model the
//!    frontend produces, choosing the reader by the document's `formatVersion`.
//! 2. [`raise`] turns a resolved module back into the Elm AST, deciding how each
//!    fully qualified name is written and what has to be imported for that
//!    spelling to mean it.
//! 3. [`print`] writes the AST out in `elm-format`'s shape.
//!
//! So the generated source cannot depend on which IR version the document was
//! written in — `tests/backend.rs` compares the two, byte for byte — and it
//! reads back as the document it came from, which `tests/roundtrip.rs` checks by
//! compiling it again.

pub mod decode;
pub mod print;
pub mod raise;

use morphir_extension_sdk::{
    Artifact, Diagnostic, DiagnosticSeverity, GenerateRequest, GenerateResult,
};

use crate::prelude;

/// A request this backend cannot act on.
pub const REQUEST: &str = "ELM_REQUEST";

/// The value definitions a document held and this backend did not write.
pub const VALUE_SKIPPED: &str = "ELM_VALUE_SKIPPED";

/// The target this backend generates.
pub const TARGET: &str = "elm";

/// Generates Elm for every module a distribution defines.
///
/// One artifact per module, at `src/<Module/Path>.elm`. Value definitions are
/// counted and reported once as a warning; a construct with no Elm form is
/// reported, left out, and makes the result unsuccessful — the artifacts for
/// the modules that did generate are still returned. A document this backend
/// cannot read at all fails with nothing to write.
pub fn generate(request: GenerateRequest) -> GenerateResult {
    if request.target != TARGET {
        return refused(format!(
            "this extension generates `{TARGET}`; the request asks for `{}`",
            request.target
        ));
    }

    let prelude = match prelude::from_option(request.options.get("elmPrelude")) {
        Ok(prelude) => prelude,
        Err(reason) => return refused(format!("invalid `elmPrelude` option: {reason}")),
    };

    let decoded = match decode::decode(&request.ir) {
        Ok(decoded) => decoded,
        Err(diagnostics) => {
            return GenerateResult {
                success: false,
                artifacts: Vec::new(),
                diagnostics,
            };
        }
    };

    let artifacts = decoded
        .modules
        .iter()
        .map(|module| Artifact {
            path: format!("src/{}.elm", module.name.join("/")),
            content: print::print(&raise::raise(&decoded.package, module, &prelude)),
            binary: false,
        })
        .collect();

    let mut diagnostics = decoded.diagnostics;
    if decoded.omitted_values > 0 {
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Warning,
            code: Some(VALUE_SKIPPED.to_string()),
            message: format!(
                "omitted {} value definitions: this backend generates types only",
                decoded.omitted_values
            ),
            location: None,
            related: Vec::new(),
        });
    }

    // A document that would not decode at all has already returned above. What
    // is left is a document that decoded with something in it this backend
    // could not write: the artifacts for every other module are still worth
    // handing back, but the run did not generate what it was asked to, so it is
    // not a success.
    let success = !diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error);

    GenerateResult {
        success,
        artifacts,
        diagnostics,
    }
}

fn refused(message: String) -> GenerateResult {
    GenerateResult {
        success: false,
        artifacts: Vec::new(),
        diagnostics: vec![Diagnostic {
            severity: DiagnosticSeverity::Error,
            code: Some(REQUEST.to_string()),
            message,
            location: None,
            related: Vec::new(),
        }],
    }
}

//! The request boundary: everything the compiler needs is checked once, here,
//! so that the rest of the frontend works with values it can trust.
//!
//! A request that this module rejects produces a single `ELM_REQUEST`
//! diagnostic and no module results: nothing about the documents was even
//! looked at, so there is nothing per-module to report.

use morphir_extension_sdk::{CompileRequest, Diagnostic, DiagnosticSeverity};

use crate::prelude::{self, Prelude};
use crate::resolved::Access;

/// The diagnostic code for a request this extension cannot act on.
pub const REQUEST: &str = "ELM_REQUEST";

/// The IR versions this frontend writes.
pub const IR_VERSIONS: [&str; 2] = ["3", "4"];

/// A request that has been checked, with the derived values the compiler uses.
pub struct Validated {
    /// `"3"` or `"4"`.
    pub ir_version: String,
    /// The request's `typesOnly` flag. This frontend lowers type declarations
    /// only, so it is recorded rather than acted on.
    pub types_only: bool,
    pub prelude: Prelude,
    /// The package path, one segment per `/`- or `.`-separated part.
    pub package: Vec<String>,
    /// The exact public module list, when the request states one.
    pub exposed: Option<Vec<String>>,
}

/// An error diagnostic about the request itself, with no source location.
pub fn request_error(message: impl Into<String>) -> Diagnostic {
    request_diagnostic(DiagnosticSeverity::Error, message)
}

/// A warning about the request itself, with no source location.
pub fn request_warning(message: impl Into<String>) -> Diagnostic {
    request_diagnostic(DiagnosticSeverity::Warning, message)
}

fn request_diagnostic(severity: DiagnosticSeverity, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        severity,
        code: Some(REQUEST.into()),
        message: message.into(),
        location: None,
        related: Vec::new(),
    }
}

/// Checks the request and derives the prelude and package path from it.
pub fn validate(request: &CompileRequest) -> Result<Validated, Diagnostic> {
    if request.language_id != "elm" {
        return Err(request_error(format!(
            "this extension compiles `elm`; the request asks for `{}`",
            request.language_id
        )));
    }

    let ir_version = request.options.ir_version.clone();
    if !IR_VERSIONS.contains(&ir_version.as_str()) {
        return Err(request_error(format!(
            "unsupported `irVersion` `{ir_version}`: this extension writes Morphir IR {}",
            IR_VERSIONS.join(" or ")
        )));
    }

    let prelude = prelude::from_option(request.options.extra.get("elmPrelude"))
        .map_err(|reason| request_error(format!("invalid `elmPrelude` option: {reason}")))?;

    let package = package_path(&request.package.name);
    if package.is_empty() {
        return Err(request_error(
            "the request names no package: `package.name` is empty",
        ));
    }

    Ok(Validated {
        ir_version,
        types_only: request.options.types_only,
        prelude,
        package,
        exposed: request.package.exposed_modules.clone(),
    })
}

/// The package path a Morphir package name spells. Both the `local/example`
/// and the `My.Package` spellings are accepted, and both separators split.
pub fn package_path(name: &str) -> Vec<String> {
    name.split(['/', '.'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect()
}

/// A module is public when the request exposes every module, or names this one.
pub fn module_access(exposed: Option<&[String]>, dotted_name: &str) -> Access {
    match exposed {
        None => Access::Public,
        Some(names) if names.iter().any(|name| name == dotted_name) => Access::Public,
        Some(_) => Access::Private,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_package_name_splits_on_both_separators() {
        assert_eq!(package_path("local/example"), ["local", "example"]);
        assert_eq!(package_path("My.Package"), ["My", "Package"]);
        assert_eq!(package_path("morphir/sdk.core"), ["morphir", "sdk", "core"]);
        assert!(package_path("  ").is_empty());
    }

    #[test]
    fn an_unlisted_module_is_private_only_when_a_list_is_given() {
        assert_eq!(module_access(None, "My.Types"), Access::Public);
        assert_eq!(
            module_access(Some(&["My.Types".to_string()]), "My.Types"),
            Access::Public
        );
        assert_eq!(module_access(Some(&[]), "My.Types"), Access::Private);
    }
}

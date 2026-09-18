//! The request boundary: everything the compiler needs is checked once, here,
//! so that the rest of the frontend works with values it can trust.
//!
//! A request that this module rejects produces a single `ELM_REQUEST`
//! diagnostic and no module results: nothing about the documents was even
//! looked at, so there is nothing per-module to report.

use morphir_extension_sdk::{CompileRequest, Diagnostic, DiagnosticSeverity};
use serde::Serialize;

use crate::digest::sha256_hex;
use crate::frontend::resolve::DependencyInterface;
use crate::names;
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
    /// The prelude names are resolved against.
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

/// The member of [`IR_VERSIONS`] a request names. A host sends either the major version or the
/// full first release of it (`4` or `4.0.0`), as the Morphir CLI does.
fn normalized_ir_version(requested: &str) -> Option<&'static str> {
    IR_VERSIONS
        .into_iter()
        .find(|version| requested == *version || requested == format!("{version}.0.0"))
}

/// Checks the request and derives the prelude and package path from it.
pub fn validate(request: &CompileRequest) -> Result<Validated, Diagnostic> {
    if request.language_id != "elm" {
        return Err(request_error(format!(
            "this extension compiles `elm`; the request asks for `{}`",
            request.language_id
        )));
    }

    let Some(ir_version) = normalized_ir_version(&request.options.ir_version) else {
        return Err(request_error(format!(
            "unsupported `irVersion` `{}`: this extension writes Morphir IR {}",
            request.options.ir_version,
            IR_VERSIONS.join(" or ")
        )));
    };
    let ir_version = ir_version.to_string();

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

/// The identity of everything a module's compiled form depends on besides its
/// own source: the IR version being written, the `typesOnly` flag, the prelude
/// names resolve against, the package the request compiles under, and the
/// public interfaces the request's dependency distributions supply.
///
/// This is the value a baseline is scoped to. A run is allowed to reuse a
/// module only when it is compiling under the very same context, because a
/// reused module's references were resolved under the old one — a dependency
/// that lost a type, a different prelude, or a renamed package would
/// otherwise be invisible to the reuse decision. Without the package path, a
/// package renamed between two runs of the very same sources would reuse
/// baseline IR whose FQNames and module keys still named the old package.
///
/// The digest is computed over a canonical JSON rendering with the
/// dependencies sorted end-to-end (by package path, then by module) and each
/// package's modules sorted by name, so two runs supplied the same
/// dependencies in a different order agree.
pub fn context_digest(validated: &Validated, dependencies: &[DependencyInterface]) -> String {
    let mut dependencies: Vec<DependencyIdentity> = dependencies
        .iter()
        .map(|dependency| {
            let mut modules: Vec<(Vec<String>, String)> = dependency
                .modules
                .iter()
                .map(|module| (module.name.clone(), module.digest()))
                .collect();
            modules.sort();
            DependencyIdentity {
                package: dependency.package.clone(),
                modules,
            }
        })
        .collect();
    dependencies.sort();

    let identity = ContextIdentity {
        ir_version: &validated.ir_version,
        types_only: validated.types_only,
        prelude_digest: validated.prelude.digest(),
        package: &validated.package,
        dependencies,
    };
    let json = serde_json::to_vec(&identity).expect("the compile context serializes to JSON");
    sha256_hex(&json)
}

/// The canonical shape [`context_digest`] hashes.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContextIdentity<'a> {
    ir_version: &'a str,
    types_only: bool,
    prelude_digest: String,
    package: &'a [String],
    dependencies: Vec<DependencyIdentity>,
}

/// One dependency package's identity: its path and its modules' interfaces.
///
/// `Ord` is derived, rather than comparing only the package path, so that
/// [`context_digest`] can sort dependencies into a fully deterministic order:
/// two runs supplied the same dependencies in a different order — or even two
/// same-named dependencies with their modules given in a different order —
/// hash identically.
#[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
struct DependencyIdentity {
    package: Vec<String>,
    modules: Vec<(Vec<String>, String)>,
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

/// The IR module path an Elm module name spells, with the package path
/// stripped when the name starts with it.
///
/// This is morphir-elm's rule (`Morphir.Elm.Frontend`, `List.drop (List.length
/// currentPackagePath)`): a package `My.Package` holding `My.Package.Foo.Bar`
/// publishes the module as `Foo.Bar`, so a dependent that imports
/// `My.Package.Foo.Bar` finds it by the same package-path prefix match the
/// resolver already does for dependency packages. Without the strip, a package
/// this frontend compiles could not be imported under its natural name.
///
/// Segments are compared in their [`crate::names`] spelling, because that is
/// the only spelling a Morphir document keeps. A name that does not start with
/// the package path (package `local/example`, module `Example`) is its own IR
/// module path, and so is one that *is* the package path exactly, since a
/// module path cannot be empty.
pub fn relative_module(package: &[String], module: &[String]) -> Vec<String> {
    if module.len() <= package.len() {
        return module.to_vec();
    }
    let matches = package
        .iter()
        .zip(module)
        .all(|(left, right)| names::type_spelling(left) == names::type_spelling(right));
    if matches {
        module[package.len()..].to_vec()
    } else {
        module.to_vec()
    }
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
    fn a_full_release_spelling_names_the_same_ir_version() {
        assert_eq!(normalized_ir_version("3"), Some("3"));
        assert_eq!(normalized_ir_version("3.0.0"), Some("3"));
        assert_eq!(normalized_ir_version("4"), Some("4"));
        assert_eq!(normalized_ir_version("4.0.0"), Some("4"));
        assert_eq!(normalized_ir_version("4.1.0"), None);
        assert_eq!(normalized_ir_version("5"), None);
    }

    #[test]
    fn a_package_name_splits_on_both_separators() {
        assert_eq!(package_path("local/example"), ["local", "example"]);
        assert_eq!(package_path("My.Package"), ["My", "Package"]);
        assert_eq!(package_path("morphir/sdk.core"), ["morphir", "sdk", "core"]);
        assert!(package_path("  ").is_empty());
    }

    #[test]
    fn the_package_path_is_stripped_from_a_module_that_starts_with_it() {
        let path = |name: &str| package_path(name);
        assert_eq!(
            relative_module(&path("My.Package"), &path("My.Package.Foo.Bar")),
            ["Foo", "Bar"]
        );
        // Not a prefix: the module keeps its whole name.
        assert_eq!(
            relative_module(&path("local/example"), &path("Example")),
            ["Example"]
        );
        // A module path cannot be empty, so an exact match is not stripped.
        assert_eq!(
            relative_module(&path("My.Package"), &path("My.Package")),
            ["My", "Package"]
        );
        // The comparison is on the words a document keeps, not on the letters.
        assert_eq!(
            relative_module(&path("local/example"), &path("Local.Example.Thing")),
            ["Thing"]
        );
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

    /// Everything the identity is meant to cover moves the digest, and nothing
    /// else does — including the order the dependencies arrived in.
    #[test]
    fn the_context_digest_covers_the_version_the_prelude_the_package_and_the_dependencies() {
        use crate::resolved::{Interface, InterfaceType};

        let validated = |ir_version: &str, prelude_id: &str, package: &str| Validated {
            ir_version: ir_version.to_string(),
            types_only: false,
            prelude: prelude::from_option(Some(&serde_json::json!(prelude_id)))
                .expect("a known prelude"),
            package: package_path(package),
            exposed: None,
        };
        let interface = |type_name: &str| Interface {
            name: vec!["Types".into()],
            types: vec![InterfaceType {
                name: type_name.into(),
                params: vec![],
                alias: None,
                constructors: None,
            }],
        };
        let dependency = |package: &str, type_name: &str| DependencyInterface {
            package: vec![package.to_string()],
            modules: vec![interface(type_name)],
        };

        let base = context_digest(&validated("3", "elm-core", "My"), &[]);
        assert!(base.starts_with("sha256:"));
        assert_ne!(base, context_digest(&validated("4", "elm-core", "My"), &[]));
        assert_ne!(base, context_digest(&validated("3", "none", "My"), &[]));
        assert_ne!(
            base,
            context_digest(&validated("3", "elm-core", "Other"), &[]),
            "compiling identical sources under a renamed package is a different context"
        );

        let with = context_digest(
            &validated("3", "elm-core", "My"),
            &[dependency("Acme", "T"), dependency("Beta", "U")],
        );
        assert_ne!(base, with);
        assert_eq!(
            with,
            context_digest(
                &validated("3", "elm-core", "My"),
                &[dependency("Beta", "U"), dependency("Acme", "T")]
            ),
            "the order dependencies arrive in is not part of the identity"
        );
        assert_ne!(
            with,
            context_digest(
                &validated("3", "elm-core", "My"),
                &[dependency("Acme", "T"), dependency("Beta", "Renamed")]
            ),
            "a dependency whose interface changed is a different context"
        );
    }

    /// [`DependencyIdentity`] sorts on its whole value, not only on the package
    /// path, so two dependencies that arrived in a different order still hash
    /// the same even when a package-only sort could not have told them apart on
    /// its own (a stable sort over equal keys keeps the input order).
    #[test]
    fn two_dependencies_with_the_same_package_hash_the_same_in_either_order() {
        use crate::resolved::{Interface, InterfaceType};

        let validated = || Validated {
            ir_version: "3".to_string(),
            types_only: false,
            prelude: prelude::from_option(Some(&serde_json::json!("elm-core")))
                .expect("a known prelude"),
            package: vec!["My".into()],
            exposed: None,
        };
        let interface = |module_name: &str, type_name: &str| Interface {
            name: vec![module_name.into()],
            types: vec![InterfaceType {
                name: type_name.into(),
                params: vec![],
                alias: None,
                constructors: None,
            }],
        };
        // Two same-named "Acme" dependencies, standing in for two module
        // interfaces of the one package supplied in a different order.
        let dependency = |module_name: &str, type_name: &str| DependencyInterface {
            package: vec!["Acme".to_string()],
            modules: vec![interface(module_name, type_name)],
        };

        let forward = context_digest(
            &validated(),
            &[dependency("Alpha", "T"), dependency("Beta", "U")],
        );
        let backward = context_digest(
            &validated(),
            &[dependency("Beta", "U"), dependency("Alpha", "T")],
        );
        assert_eq!(
            forward, backward,
            "two same-package dependencies given in either order hash the same"
        );
    }
}

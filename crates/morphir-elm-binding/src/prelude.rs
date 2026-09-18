//! Data-driven Elm prelude: implicit imports, SDK module aliases, and
//! platform package interfaces used by the resolver.
//!
//! The `elm-core` prelude is pinned against morphir-elm's
//! `Morphir.Elm.IncrementalResolve` module (see `tests/prelude.rs`).

use serde::{Deserialize, Serialize};

use crate::digest::sha256_hex;

/// The built-in `elm-core` prelude, embedded at compile time.
pub const ELM_CORE: &str = include_str!("../preludes/elm-core.toml");

/// A data-driven Elm prelude: implicit imports, SDK module aliases, and
/// platform package interfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prelude {
    /// The prelude's identifier, named in diagnostics.
    pub id: String,
    /// Imports every module is compiled as if it had written.
    #[serde(default)]
    pub implicit_import: Vec<ImplicitImport>,
    /// Source-to-target module path mappings.
    #[serde(default)]
    pub module_alias: Vec<ModuleAlias>,
    /// The platform packages this prelude makes available.
    #[serde(default)]
    pub package: Vec<PlatformPackage>,
}

/// An implicit import, as if every module in the frontend started with
/// `import <module> exposing (<exposing>)`. Entries in `exposing` are either
/// a bare name (`"Int"`), a type with constructors exposed (`"Order(..)"`),
/// or `".."` meaning everything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImplicitImport {
    /// The dotted module path that is imported.
    pub module: String,
    /// What the import exposes unqualified.
    #[serde(default)]
    pub exposing: Vec<String>,
}

/// Maps a dotted source module path (as written in Elm source) to a dotted
/// target module path (the module it actually resolves to).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleAlias {
    /// The dotted path a reader writes.
    pub source: String,
    /// The dotted path it resolves to.
    pub target: String,
}

/// A platform package (e.g. `Morphir.SDK`) and the modules it provides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformPackage {
    /// The package's dotted name.
    pub name: String,
    /// The modules it provides.
    #[serde(default)]
    pub module: Vec<PlatformModule>,
}

/// A module within a platform package, and the types it declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformModule {
    /// The module's dotted name, relative to nothing: it is written in full.
    pub name: String,
    /// The types the module declares.
    #[serde(default)]
    pub types: Vec<PlatformType>,
}

/// A type declared by a platform module. `constructors` is non-empty only
/// for custom types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformType {
    /// The type's name.
    pub name: String,
    /// Its constructors, for a custom type.
    #[serde(default)]
    pub constructors: Vec<String>,
}

/// Looks up a built-in prelude by id: `"elm-core"` or `"none"`.
pub fn builtin(id: &str) -> Option<Prelude> {
    match id {
        "elm-core" => Some(parse_toml(ELM_CORE).expect("bundled elm-core.toml must parse")),
        "none" => Some(Prelude {
            id: "none".to_string(),
            implicit_import: Vec::new(),
            module_alias: Vec::new(),
            package: Vec::new(),
        }),
        _ => None,
    }
}

/// Resolves the `elmPrelude` extension option into a [`Prelude`].
///
/// - `None` selects the default `elm-core` prelude.
/// - A JSON string selects a built-in prelude by id.
/// - A JSON object is deserialized directly as an inline [`Prelude`].
pub fn from_option(value: Option<&serde_json::Value>) -> Result<Prelude, String> {
    match value {
        None => Ok(builtin("elm-core").expect("elm-core is always available")),
        Some(serde_json::Value::String(s)) => {
            builtin(s).ok_or_else(|| format!("unknown prelude `{s}`"))
        }
        Some(v @ serde_json::Value::Object(_)) => {
            serde_json::from_value(v.clone()).map_err(|e| format!("invalid prelude object: {e}"))
        }
        Some(other) => Err(format!(
            "invalid `elmPrelude` option: expected a string or object, got {other}"
        )),
    }
}

/// Parses a TOML prelude document.
pub fn parse_toml(text: &str) -> Result<Prelude, String> {
    toml::from_str(text).map_err(|e| format!("invalid prelude TOML: {e}"))
}

impl Prelude {
    /// A content digest of this prelude, stable across process runs.
    pub fn digest(&self) -> String {
        let json = serde_json::to_vec(self).expect("Prelude serializes to JSON");
        sha256_hex(&json)
    }

    /// Resolves a dotted module path written in source to the dotted path it
    /// actually refers to, per `module_alias`.
    pub fn alias_for(&self, module: &[String]) -> Option<Vec<String>> {
        self.module_alias.iter().find_map(|alias| {
            let matches = alias
                .source
                .split('.')
                .eq(module.iter().map(String::as_str));
            if matches {
                Some(alias.target.split('.').map(str::to_string).collect())
            } else {
                None
            }
        })
    }

    /// The inverse of [`Prelude::alias_for`]: given a resolved dotted module
    /// path, finds the source path that aliases to it.
    pub fn unalias(&self, target: &[String]) -> Option<Vec<String>> {
        self.module_alias.iter().find_map(|alias| {
            let matches = alias
                .target
                .split('.')
                .eq(target.iter().map(String::as_str));
            if matches {
                Some(alias.source.split('.').map(str::to_string).collect())
            } else {
                None
            }
        })
    }

    /// Finds the platform package and module whose combined dotted name
    /// matches `module` (package name segments as a prefix, remaining
    /// segments joined with `.` as the module name).
    ///
    /// When more than one package's name is a prefix of `module` (e.g. one
    /// prelude configuring both `Morphir` and `Morphir.SDK` as packages),
    /// the package with the longest (most specific) matching prefix wins, so
    /// candidates are checked longest-prefix-first.
    pub fn platform_module(
        &self,
        module: &[String],
    ) -> Option<(&PlatformPackage, &PlatformModule)> {
        let mut candidates: Vec<&PlatformPackage> = self.package.iter().collect();
        candidates.sort_by_key(|pkg| std::cmp::Reverse(pkg.name.split('.').count()));
        candidates.into_iter().find_map(|pkg| {
            let pkg_segments: Vec<&str> = pkg.name.split('.').collect();
            if module.len() <= pkg_segments.len() {
                return None;
            }
            let prefix_matches = module[..pkg_segments.len()]
                .iter()
                .map(String::as_str)
                .eq(pkg_segments.iter().copied());
            if !prefix_matches {
                return None;
            }
            let remainder = module[pkg_segments.len()..].join(".");
            pkg.module
                .iter()
                .find(|m| m.name == remainder)
                .map(|m| (pkg, m))
        })
    }
}

//! The document tree's logical path grammar: classification, escaping, and the physical
//! boundary.
//!
//! A logical path is a POSIX path with no extension. The seven forms a v4 document tree spells
//! are `manifest`, `pkg/<package>/<module>/module`, `pkg/<package>/<module>/<stem>.type`,
//! `pkg/<package>/<module>/<stem>.value`, and their `deps/<package>/@<version>/...` counterparts
//! (the v4 model carries no package version, so the version slot is always the bare `@`,
//! decision 0015). `.type` and `.value` are part of the logical name, not filename extensions.
//!
//! This mirrors `IR/src/layout/paths.ts` in `ecosystem/morphir-typescript`, function for
//! function; see `.dev/docs/superpowers/maps/2026-09-17-reference-tree-layout-map.md` section 1.

use crate::naming::{self, PackageName, Path};

use super::Profile;

/// The distribution manifest's logical path.
pub const MANIFEST: &str = "manifest";

/// The bare version segment a dependency directory carries, because the v4 model has no package
/// version to hold (decision 0015).
pub const VERSION_SLOT: &str = "@";

/// Which of the tree's two roots a path or directory sits under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Root {
    /// The tree's own package.
    Pkg,
    /// A dependency package.
    Deps,
}

impl Root {
    /// The root's one wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Root::Pkg => "pkg",
            Root::Deps => "deps",
        }
    }
}

/// The kind a physical or logical path names, and what `classify` can read out of it.
///
/// `dir` never includes the root: every builder that needs the root back adds it through
/// [`module_dir_prefix`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathKind {
    Manifest,
    Module {
        root: Root,
        dir: String,
    },
    Type {
        root: Root,
        dir: String,
        stem: String,
    },
    Value {
        root: Root,
        dir: String,
        stem: String,
    },
    Other,
}

/// Which of the two node-file kinds a path names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeFileKind {
    Type,
    Value,
}

impl NodeFileKind {
    /// The kind's one wire spelling: the leaf's extension-like suffix.
    pub fn as_str(self) -> &'static str {
        match self {
            NodeFileKind::Type => "type",
            NodeFileKind::Value => "value",
        }
    }
}

/// Classifies a logical path the way the reference's `classify` does, line for line: the manifest
/// path first, then the root, then the leaf.
///
/// `classify` never validates a stem against the escaped-stem grammar — `pkg/a/b/X Y.type`
/// classifies as a type with stem `X Y`, and the leaf match is greedy, so `a.type.value` is a
/// value with stem `a.type`.
pub fn classify(logical: &str) -> PathKind {
    if logical == MANIFEST {
        return PathKind::Manifest;
    }

    let parts: Vec<&str> = logical.split('/').collect();
    let root = match parts.first() {
        Some(&"pkg") => Root::Pkg,
        Some(&"deps") => Root::Deps,
        _ => return PathKind::Other,
    };

    let rest = &parts[1..];
    let Some((leaf, dir_parts)) = rest.split_last() else {
        return PathKind::Other;
    };
    let dir = dir_parts.join("/");

    if *leaf == "module" {
        return PathKind::Module { root, dir };
    }

    match split_definition_leaf(leaf) {
        Some((stem, NodeFileKind::Type)) => PathKind::Type {
            root,
            dir,
            stem: stem.to_string(),
        },
        Some((stem, NodeFileKind::Value)) => PathKind::Value {
            root,
            dir,
            stem: stem.to_string(),
        },
        None => PathKind::Other,
    }
}

/// The reference's `DEFINITION_LEAF` regex, `^(.+)\.(type|value)$`: a non-empty stem followed by
/// a literal `.type` or `.value` at the very end of the leaf. The pattern is anchored at both
/// ends, so there is exactly one way to split a leaf that matches it — greediness only matters in
/// that `a.type.value` keeps `.type` as part of the stem rather than splitting on it.
fn split_definition_leaf(leaf: &str) -> Option<(&str, NodeFileKind)> {
    if let Some(stem) = leaf.strip_suffix(".value")
        && !stem.is_empty()
    {
        return Some((stem, NodeFileKind::Value));
    }
    if let Some(stem) = leaf.strip_suffix(".type")
        && !stem.is_empty()
    {
        return Some((stem, NodeFileKind::Type));
    }
    None
}

/// The physical path a profile writes a logical path under: the logical path plus the profile's
/// extension.
pub fn to_physical(logical: &str, profile: Profile) -> String {
    format!("{logical}{}", profile.extension())
}

/// The logical path a physical name carries, or `None` when its extension is not one the tree
/// recognizes.
///
/// Normalizes `\` to `/` before stripping, so a Windows-style relative path resolves the same way
/// a POSIX one does. `.yml` is recognized here (read-only) even though [`Profile`] never writes
/// it.
pub fn from_physical(physical: &str) -> Option<String> {
    let normalized = physical.replace('\\', "/");
    for ext in [".json", ".yaml", ".yml"] {
        if let Some(stripped) = normalized.strip_suffix(ext) {
            return Some(stripped.to_string());
        }
    }
    None
}

/// The escaped directory a package's files sit under: the package path's escaped stems, joined
/// by `/`, with a trailing bare `@` under `deps/`.
pub fn package_dir(root: Root, package: &PackageName) -> String {
    let escaped = naming::escaped_path(package.as_path());
    match root {
        Root::Pkg => escaped,
        Root::Deps => format!("{escaped}/{VERSION_SLOT}"),
    }
}

/// The escaped directory a module's files sit under: the package directory, then the module
/// path's escaped stems.
pub fn module_dir(root: Root, package: &PackageName, module: &Path) -> String {
    format!(
        "{}/{}",
        package_dir(root, package),
        naming::escaped_path(module)
    )
}

/// The physical-path prefix every file under a module directory shares: `<root>/<dir>/`.
pub fn module_dir_prefix(root: Root, dir: &str) -> String {
    format!("{}/{dir}/", root.as_str())
}

/// The logical path of a module's manifest.
pub fn module_manifest_path(root: Root, dir: &str) -> String {
    format!("{}module", module_dir_prefix(root, dir))
}

/// The logical path of one type's or value's node file.
pub fn node_file_path(root: Root, dir: &str, stem: &str, kind: NodeFileKind) -> String {
    format!("{}{stem}.{}", module_dir_prefix(root, dir), kind.as_str())
}

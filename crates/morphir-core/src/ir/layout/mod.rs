//! The document-tree layout: the storage-profile boundary, the logical path grammar, and file
//! stems.
//!
//! Mirrors `IR/src/layout/{index,paths,stems}.ts` in `ecosystem/morphir-typescript`; see
//! `.dev/docs/superpowers/maps/2026-09-17-reference-tree-layout-map.md`. This module supplies
//! the paths and stems a tree reader and a tree writer are both built on (tasks 5 and 6); it does
//! not itself read or write a tree.

pub mod paths;
pub mod stems;

pub use paths::{
    MANIFEST, NodeFileKind, PathKind, Root, VERSION_SLOT, classify, from_physical, module_dir,
    module_dir_prefix, module_manifest_path, node_file_path, package_dir, to_physical,
};
pub use stems::{StemResult, stem_for};

use crate::ir::Diagnostic;

/// The two storage profiles a document tree can be laid out under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Json,
    Yaml,
}

impl Profile {
    /// The profile's name, as the adapter protocol and the manifest's `profile` field spell it.
    pub fn name(self) -> &'static str {
        match self {
            Profile::Json => "json",
            Profile::Yaml => "yaml",
        }
    }

    /// The extension [`to_physical`] appends for this profile.
    pub fn extension(self) -> &'static str {
        match self {
            Profile::Json => ".json",
            Profile::Yaml => ".yaml",
        }
    }

    /// Reads one file's text as a JSON value tree under this profile.
    pub fn read(self, text: &str) -> Result<serde_json::Value, Diagnostic> {
        match self {
            Profile::Json => crate::ir::json::read(text),
            Profile::Yaml => crate::ir::yaml::read(text),
        }
    }

    /// Writes a JSON value tree as this profile's canonical text.
    ///
    /// JSON writes one line with no trailing newline; YAML writes exactly one trailing `\n`.
    pub fn write(self, value: &serde_json::Value) -> String {
        match self {
            Profile::Json => crate::ir::json::write_canonical(value),
            Profile::Yaml => crate::ir::yaml::write_canonical(value),
        }
    }
}

/// An in-memory document tree: logical path to file text, ordered so a byte-for-byte comparison
/// of two trees does not depend on insertion order.
pub type Tree = std::collections::BTreeMap<String, String>;

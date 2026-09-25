//! The four files a document tree is made of, as ordinary version 4 nodes.
//!
//! A document tree spells one distribution across a set of files: a distribution manifest at
//! `manifest`, one module manifest per module at `.../module`, and one node file per type or value
//! at `.../<stem>.type` or `.../<stem>.value`. Each of them is a node in its own right — it
//! repeats the format version at its root, it is checked against the same support table a whole
//! document is checked against, and it has exactly one canonical spelling — so the models live
//! here beside the rest of the v4 model and the layout that assembles them reads and writes them
//! the way the adapter reads and writes any other node.
//!
//! The decoders are in [`super::serde_document`], with the rest of the cursor-carrying decoders;
//! the canonical encoders are the `Serialize` impls below, whose member order is the reference
//! writers' order and is pinned by `crates/morphir-core/tests/v4_tree_files.rs`.
//!
//! What a file leaves out is as much of the contract as what it writes: a member that only repeats
//! a default — an empty `dependencies`, a `Public` `access`, entry points on a distribution that
//! has none — is not written, so the shape of a file does not depend on how much is in it. The
//! reserved `$meta` is stripped on the way in and therefore never written back (decision 0014).

use indexmap::IndexMap;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::access::{Access, AccessControlled};
use super::distribution::EntryPoints;
use super::module::{Documentation, Documented};
use super::types::{TypeDefinition, TypeSpecification};
use super::value::{ValueDefinition, ValueSpecification};
use super::{FormatVersion, serde_document};
use crate::naming::{ModuleName, Name, PackageName};

/// The smallest `pathBudget` a tree can be laid out under (decision 0012).
pub const MIN_PATH_BUDGET: u32 = 64;

/// Decision 0012's escaped stem: lowercase words joined by hyphens, an optional leading or
/// trailing underscore for the escape forms, and the eight hex digits a truncated stem carries
/// after `__`.
///
/// Written out once here and matched by [`is_escaped_stem`], which is the only reader of it: the
/// reference keeps two copies of this regular expression (`layout/stems.ts` and
/// `versions/v4/read-tree-files.ts`) and that duplication is a known defect, not a thing to port.
pub const FILE_STEM_PATTERN: &str = r"^_?[a-z0-9]+(-_?[a-z0-9]+)*(__[0-9a-f]{8})?_?$";

/// Whether `stem` is an escaped file stem, matching [`FILE_STEM_PATTERN`].
///
/// Hand-written rather than compiled, because the pattern is a fixed one this crate owns and a
/// regular-expression dependency for one match is a dependency for one match. The scan is
/// deterministic: `[a-z0-9]+` never crosses a `-` or a `_`, so each run ends at a character the
/// next part of the grammar decides about, and nothing has to be reconsidered.
pub fn is_escaped_stem(stem: &str) -> bool {
    /// `__` plus eight hex digits.
    const HASH_SUFFIX_LEN: usize = 10;

    fn alphanumeric(byte: u8) -> bool {
        byte.is_ascii_lowercase() || byte.is_ascii_digit()
    }

    /// The hash suffix is lowercase hex, which `is_ascii_hexdigit` would also accept uppercase.
    fn lowercase_hex(byte: &u8) -> bool {
        byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
    }

    // The grammar is ASCII, so a multi-byte character can only make a run end early and then fail
    // the "consumed it all" test at the end.
    let bytes = stem.as_bytes();
    let mut at = 0;

    // `_?`
    if bytes.first() == Some(&b'_') {
        at += 1;
    }
    // `[a-z0-9]+`
    let run = at;
    while at < bytes.len() && alphanumeric(bytes[at]) {
        at += 1;
    }
    if at == run {
        return false;
    }
    // `(-_?[a-z0-9]+)*`
    while bytes.get(at) == Some(&b'-') {
        let mut next = at + 1;
        if bytes.get(next) == Some(&b'_') {
            next += 1;
        }
        let run = next;
        while next < bytes.len() && alphanumeric(bytes[next]) {
            next += 1;
        }
        if next == run {
            // A hyphen with no segment after it: leave it unconsumed so the length check below
            // refuses the whole stem.
            break;
        }
        at = next;
    }
    // `(__[0-9a-f]{8})?`
    if bytes.len() - at >= HASH_SUFFIX_LEN
        && bytes[at] == b'_'
        && bytes[at + 1] == b'_'
        && bytes[at + 2..at + HASH_SUFFIX_LEN]
            .iter()
            .all(lowercase_hex)
    {
        at += HASH_SUFFIX_LEN;
    }
    // `_?$`
    if bytes.get(at) == Some(&b'_') {
        at += 1;
    }
    at == bytes.len()
}

// =============================================================================
// The distribution manifest
// =============================================================================

/// Which of the three distributions a tree holds, as the manifest names it.
///
/// This is the manifest's own spelling of the kind, not [`super::Distribution`]: a manifest names
/// the kind and lists its dependencies by name, while the bodies live in the files under `pkg/`
/// and `deps/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistributionKind {
    Library,
    Specs,
    Application,
}

impl DistributionKind {
    /// The kind's one wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Library => "Library",
            Self::Specs => "Specs",
            Self::Application => "Application",
        }
    }

    /// The kind a manifest's `distribution` string names, or `None` if it names none.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "Library" => Some(Self::Library),
            "Specs" => Some(Self::Specs),
            "Application" => Some(Self::Application),
            _ => None,
        }
    }
}

impl Serialize for DistributionKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// A document tree's root file: which distribution the tree holds, whose package it is, the path
/// budget it was laid out under, and the names of its dependencies.
#[derive(Debug, Clone, PartialEq)]
pub struct DistributionManifestFile {
    pub format_version: FormatVersion,
    pub distribution: DistributionKind,
    pub package: PackageName,
    pub path_budget: u32,
    /// The dependency packages by name; their bodies live under `deps/`. Written only when
    /// non-empty.
    pub dependencies: Vec<PackageName>,
    /// An application's entry points. Refused on the other two kinds, and written only when an
    /// application has any.
    pub entry_points: EntryPoints,
}

impl Serialize for DistributionManifestFile {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("formatVersion", &self.format_version)?;
        map.serialize_entry("distribution", &self.distribution)?;
        map.serialize_entry("package", &self.package.to_canonical_string())?;
        map.serialize_entry("pathBudget", &self.path_budget)?;
        if !self.dependencies.is_empty() {
            let names: Vec<String> = self
                .dependencies
                .iter()
                .map(PackageName::to_canonical_string)
                .collect();
            map.serialize_entry("dependencies", &names)?;
        }
        if self.distribution == DistributionKind::Application && !self.entry_points.is_empty() {
            map.serialize_entry("entryPoints", &self.entry_points)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for DistributionManifestFile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        serde_document::deserialize_with(
            deserializer,
            serde_document::decode_distribution_manifest_file,
        )
    }
}

// =============================================================================
// The module manifest
// =============================================================================

/// A module manifest's `types` or `values` member, in one of the three styles it is written in.
///
/// A tree writer always emits [`ModuleEntries::Names`] — the bodies live in the node files beside
/// the manifest — but a manifest read on its own may inline them, and which of the two object
/// styles an object is read as is the caller's to say rather than a guess from the shape.
#[derive(Debug, Clone, PartialEq)]
pub enum ModuleEntries<D, S> {
    Names(Vec<Name>),
    Definitions(IndexMap<String, D>),
    Specifications(IndexMap<String, S>),
}

impl<D, S> ModuleEntries<D, S> {
    /// The canonical spelling of every name this listing names, whichever style it is in.
    pub fn listed_names(&self) -> Vec<String> {
        match self {
            Self::Names(names) => names.iter().map(Name::to_canonical_string).collect(),
            Self::Definitions(items) => items
                .keys()
                .map(|key| canonical_key(key.as_str()))
                .collect(),
            Self::Specifications(items) => items
                .keys()
                .map(|key| canonical_key(key.as_str()))
                .collect(),
        }
    }
}

/// A listing key as a canonical name, or the key itself when it does not parse as one.
///
/// An inline listing's keys are the document's own member names; an unparseable one is already
/// refused by the entry decoder, so the fallback only keeps this total.
fn canonical_key(key: &str) -> String {
    Name::from_canonical_string(key)
        .map(|name| name.to_canonical_string())
        .unwrap_or_else(|_| key.to_owned())
}

impl<D: Serialize, S: Serialize> Serialize for ModuleEntries<D, S> {
    fn serialize<Ser: Serializer>(&self, serializer: Ser) -> Result<Ser::Ok, Ser::Error> {
        match self {
            Self::Names(names) => {
                serializer.collect_seq(names.iter().map(Name::to_canonical_string))
            }
            Self::Definitions(items) => items.serialize(serializer),
            Self::Specifications(items) => items.serialize(serializer),
        }
    }
}

/// Which of the two object styles a module manifest's `types` and `values` are read as.
///
/// The tree's layout knows this from the distribution kind and the root the module sits under;
/// guessing it from the shape would make a specification that happens to look access-controlled
/// read as a definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedEntries {
    Definitions,
    Specifications,
}

/// One module's manifest: its path, its access, its documentation and what it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleManifestFile {
    pub format_version: FormatVersion,
    pub path: ModuleName,
    /// `Public` unless the file says otherwise; written only when `Private`, right after `path`.
    pub access: Access,
    /// One string, or — accepted only here — an array of lines joined with `\n`. Written as one
    /// string.
    pub doc: Option<Documentation>,
    pub types:
        ModuleEntries<AccessControlled<Documented<TypeDefinition>>, Documented<TypeSpecification>>,
    pub values: ModuleEntries<
        AccessControlled<Documented<ValueDefinition>>,
        Documented<ValueSpecification>,
    >,
    /// The names whose file stem was truncated for the path budget, each with the stem its file is
    /// under. Written only when non-empty, keyed by the canonical name.
    pub file_names: Vec<(Name, String)>,
}

impl Serialize for ModuleManifestFile {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("formatVersion", &self.format_version)?;
        map.serialize_entry("path", &self.path.to_canonical_string())?;
        if self.access == Access::Private {
            map.serialize_entry("access", "Private")?;
        }
        if let Some(doc) = &self.doc {
            map.serialize_entry("doc", doc)?;
        }
        map.serialize_entry("types", &self.types)?;
        map.serialize_entry("values", &self.values)?;
        if !self.file_names.is_empty() {
            let recorded: IndexMap<String, &str> = self
                .file_names
                .iter()
                .map(|(name, stem)| (name.to_canonical_string(), stem.as_str()))
                .collect();
            map.serialize_entry("fileNames", &recorded)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for ModuleManifestFile {
    /// A module manifest read as a bare node expects definitions, the way the reference's own
    /// node reader does: there is no tree around it to say which root it sits under.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        serde_document::deserialize_standalone_with(deserializer, |value, cursor| {
            serde_document::decode_module_manifest_file(value, cursor, ExpectedEntries::Definitions)
        })
    }
}

// =============================================================================
// The node files
// =============================================================================

/// What a node file holds: a definition, or the public face of one.
///
/// Which of the two it is decides what the file means, so a file carrying neither or both is a
/// shape error rather than a missing or an unknown member.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeFileBody<D, S> {
    Def(D),
    Spec(S),
}

/// One type's file: `<stem>.type`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDefinitionFile {
    pub format_version: FormatVersion,
    pub name: Name,
    pub body:
        NodeFileBody<AccessControlled<Documented<TypeDefinition>>, Documented<TypeSpecification>>,
}

impl Serialize for TypeDefinitionFile {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_node_file(serializer, &self.format_version, &self.name, &self.body)
    }
}

impl<'de> Deserialize<'de> for TypeDefinitionFile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        serde_document::deserialize_standalone_with(
            deserializer,
            serde_document::decode_type_definition_file,
        )
    }
}

/// One value's file: `<stem>.value`.
#[derive(Debug, Clone, PartialEq)]
pub struct ValueDefinitionFile {
    pub format_version: FormatVersion,
    pub name: Name,
    pub body:
        NodeFileBody<AccessControlled<Documented<ValueDefinition>>, Documented<ValueSpecification>>,
}

impl Serialize for ValueDefinitionFile {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_node_file(serializer, &self.format_version, &self.name, &self.body)
    }
}

impl<'de> Deserialize<'de> for ValueDefinitionFile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        serde_document::deserialize_standalone_with(
            deserializer,
            serde_document::decode_value_definition_file,
        )
    }
}

/// The member order both node files share: the format version, the name, then the one body.
fn serialize_node_file<S: Serializer, D: Serialize, Sp: Serialize>(
    serializer: S,
    format_version: &FormatVersion,
    name: &Name,
    body: &NodeFileBody<D, Sp>,
) -> Result<S::Ok, S::Error> {
    let mut map = serializer.serialize_map(Some(3))?;
    map.serialize_entry("formatVersion", format_version)?;
    map.serialize_entry("name", &name.to_canonical_string())?;
    match body {
        NodeFileBody::Def(definition) => map.serialize_entry("def", definition)?,
        NodeFileBody::Spec(specification) => map.serialize_entry("spec", specification)?,
    }
    map.end()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_escaped_stem_grammar_accepts_what_the_escape_produces() {
        for stem in [
            "user",
            "user-_id",
            "_con",
            "aux_",
            "value-in-_usd",
            "customer-relati__44a101f8",
            "c__44a101f8",
            "a-_b",
            "sku1",
        ] {
            assert!(is_escaped_stem(stem), "{stem}");
        }
    }

    /// The `expect` a module manifest is read under is not reachable through `Deserialize` — a
    /// bare node is always read as definitions — so these two live here rather than in
    /// `tests/v4_tree_files.rs`.
    fn decode_expecting(
        text: &str,
        expect: ExpectedEntries,
    ) -> Result<ModuleManifestFile, crate::ir::Diagnostic> {
        let value: serde_json::Value = serde_json::from_str(text).expect("the fixture is JSON");
        serde_document::decode_module_manifest_file(&value, "", expect)
    }

    #[test]
    fn a_manifest_read_as_specifications_reads_its_inline_entries_as_specifications() {
        let text = r#"{ "formatVersion": 4, "path": "orders", "types": { "sku": { "OpaqueTypeSpecification": {} } }, "values": [] }"#;
        let file = decode_expecting(text, ExpectedEntries::Specifications)
            .expect("an inline specification decodes");
        assert!(matches!(&file.types, ModuleEntries::Specifications(items) if items.len() == 1));
    }

    #[test]
    fn an_access_controlled_entry_where_a_specification_is_expected_is_refused() {
        let text = r#"{ "formatVersion": 4, "path": "orders", "types": { "sku": { "Public": { "OpaqueTypeSpecification": {} } } }, "values": [] }"#;
        let diagnostic = decode_expecting(text, ExpectedEntries::Specifications)
            .expect_err("a definition where a specification was expected is refused");
        assert_eq!(
            diagnostic.code,
            crate::ir::DiagnosticCode::InvalidDistributionShape
        );
        assert_eq!(diagnostic.cursor, "/types/sku");
        assert_eq!(
            diagnostic.message,
            "expected a specification, found an access-controlled definition"
        );
    }

    #[test]
    fn the_escaped_stem_grammar_refuses_everything_else() {
        for stem in [
            "",
            "_",
            "Not A Stem",
            "User",
            "user-",
            "user--id",
            "user_id",
            "a__44A101F8",
            "a__44a101f8-c",
            "a__zzzzzzzz",
            "user id",
        ] {
            assert!(!is_escaped_stem(stem), "{stem}");
        }
    }
}

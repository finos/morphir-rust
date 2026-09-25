//! Distribution types for Morphir IR V4
//!
//! This module contains the Distribution enum and related content types
//! (LibraryContent, SpecsContent, ApplicationContent).

use indexmap::IndexMap;
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};

use super::package::{PackageDefinition, PackageSpecification};
use crate::naming::PackageName;

/// A distribution, written as the single-member wrapper its kind names.
///
/// `{ "Library": { "packageName", "dependencies", "def" } }`,
/// `{ "Specs": { "packageName", "dependencies", "spec" } }` or
/// `{ "Application": { "packageName", "dependencies", "def", "entryPoints" } }`. A version 3
/// tagged array is not a version 4 distribution.
#[derive(Debug, Clone, PartialEq)]
pub enum Distribution {
    Library(LibraryContent),
    Specs(SpecsContent),
    Application(ApplicationContent),
}

impl Distribution {
    /// The package this distribution publishes.
    pub fn package_name(&self) -> &PackageName {
        match self {
            Distribution::Library(content) => &content.package_name,
            Distribution::Specs(content) => &content.package_name,
            Distribution::Application(content) => &content.package_name,
        }
    }
}

impl Serialize for Distribution {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            Distribution::Library(content) => {
                map.serialize_entry("Library", content)?;
            }
            Distribution::Specs(content) => {
                map.serialize_entry("Specs", content)?;
            }
            Distribution::Application(content) => {
                map.serialize_entry("Application", content)?;
            }
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Distribution {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        super::serde_document::deserialize_standalone_with(
            deserializer,
            super::serde_document::decode_distribution,
        )
    }
}

/// Library distribution content
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryContent {
    pub package_name: PackageName,
    pub dependencies: Dependencies,
    pub def: PackageDefinition,
}

/// Specs distribution content (public interfaces only)
///
/// A `Specs` distribution publishes a package's public face and nothing else, so it carries a
/// `spec` where a library carries a `def`; a `def` beside it is an unknown member.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecsContent {
    pub package_name: PackageName,
    pub dependencies: Dependencies,
    pub spec: PackageSpecification,
}

/// Application distribution content
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationContent {
    pub package_name: PackageName,
    pub dependencies: DefinitionDependencies,
    pub def: PackageDefinition,
    pub entry_points: EntryPoints,
}

/// Dependencies keyed by canonical package name: `{ "morphir/SDK": { "modules": {} } }`.
///
/// Decision 0011 spells the SDK `morphir/SDK`. The key is the package's canonical string, which
/// a reader checks: `morphir/sdk` is a valid name for some other package, so it is read as one
/// rather than refused.
pub type Dependencies = IndexMap<String, PackageSpecification>;

/// An application's dependencies, each a package definition (distributions-0010).
///
/// An `Application` links its dependencies statically, so it carries their definitions —
/// access-controlled modules — where a `Library` or `Specs` carries their public faces. The key
/// is read the same way: it is the dependency's canonical package name.
pub type DefinitionDependencies = IndexMap<String, PackageDefinition>;

/// Entry points for Application distribution
pub type EntryPoints = IndexMap<String, EntryPoint>;

/// Entry point definition
///
/// Keyed by a name the author chooses, holding the value it names, the kind of entry it is, and
/// an optional `doc`, which is written only when it is present.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryPoint {
    /// The entry point's target, a canonical FQName string.
    pub target: String,
    pub kind: EntryPointKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

/// Entry point kind, drawn from a fixed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryPointKind {
    Main,
    Command,
    Handler,
    Job,
    Policy,
}

impl EntryPointKind {
    /// The kinds an entry point may name, in the order the specification lists them.
    pub const ALL: &'static [EntryPointKind] = &[
        EntryPointKind::Main,
        EntryPointKind::Command,
        EntryPointKind::Handler,
        EntryPointKind::Job,
        EntryPointKind::Policy,
    ];

    /// The lowercase wire spelling of this kind.
    pub fn as_str(self) -> &'static str {
        match self {
            EntryPointKind::Main => "main",
            EntryPointKind::Command => "command",
            EntryPointKind::Handler => "handler",
            EntryPointKind::Job => "job",
            EntryPointKind::Policy => "policy",
        }
    }

    /// The kind `text` names, if it names one.
    pub fn parse(text: &str) -> Option<EntryPointKind> {
        Self::ALL.iter().copied().find(|kind| kind.as_str() == text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::naming::Path;

    #[test]
    fn test_distribution_library_serialization() {
        let dist = Distribution::Library(LibraryContent {
            package_name: PackageName::new(Path::new("my/pkg")),
            dependencies: IndexMap::new(),
            def: PackageDefinition {
                modules: IndexMap::new(),
            },
        });
        let json = serde_json::to_string(&dist).unwrap();
        assert!(json.contains("\"Library\""));
        assert!(json.contains("packageName"));
    }
}

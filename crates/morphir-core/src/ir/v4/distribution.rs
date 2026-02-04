//! Distribution types for Morphir IR V4
//!
//! This module contains the Distribution enum and related content types
//! (LibraryContent, SpecsContent, ApplicationContent).

use indexmap::IndexMap;
use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use std::fmt;

use super::package::{PackageDefinition, PackageSpecification};
use crate::naming::PackageName;

/// Distribution enum - serializes as wrapper object format
/// E.g., `{ "Library": { ... } }` or `{ "Specs": { ... } }`
#[derive(Debug, Clone, PartialEq)]
pub enum Distribution {
    Library(LibraryContent),
    Specs(SpecsContent),
    Application(ApplicationContent),
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
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(DistributionVisitor)
    }
}

struct DistributionVisitor;

impl<'de> Visitor<'de> for DistributionVisitor {
    type Value = Distribution;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a distribution object wrapper like { \"Library\": { ... } }")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Distribution, M::Error>
    where
        M: MapAccess<'de>,
    {
        let (key, value): (String, serde_json::Value) = map
            .next_entry()?
            .ok_or_else(|| de::Error::custom("expected distribution wrapper object"))?;

        match key.as_str() {
            "Library" => {
                let content: LibraryContent =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(Distribution::Library(content))
            }
            "Specs" => {
                let content: SpecsContent =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(Distribution::Specs(content))
            }
            "Application" => {
                let content: ApplicationContent =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(Distribution::Application(content))
            }
            _ => Err(de::Error::unknown_variant(
                &key,
                &["Library", "Specs", "Application"],
            )),
        }
    }
}

/// Library distribution content
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryContent {
    pub package_name: PackageName,
    pub dependencies: Dependencies,
    pub def: PackageDefinition,
}

/// Specs distribution content (public interfaces only)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecsContent {
    pub package_name: PackageName,
    pub dependencies: Dependencies,
    pub spec: PackageSpecification,
}

/// Application distribution content
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationContent {
    pub package_name: PackageName,
    pub dependencies: Dependencies,
    pub def: PackageDefinition,
    pub entry_points: EntryPoints,
}

/// Dependencies as keyed object: `{ "morphir/sdk": { modules: ... } }`
pub type Dependencies = IndexMap<String, PackageSpecification>;

/// Entry points for Application distribution
pub type EntryPoints = IndexMap<String, EntryPoint>;

/// Entry point definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryPoint {
    pub target: String, // FQName as canonical string
    pub kind: EntryPointKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

/// Entry point kind
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryPointKind {
    Main,
    Command,
    Handler,
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

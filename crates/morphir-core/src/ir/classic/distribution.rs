//! Classic IR Distribution types
//!
//! Distribution wrapper for the Classic Morphir IR format.

use serde::de::{self, IgnoredAny, SeqAccess, Visitor};
use serde::ser::{SerializeTuple, Serializer};
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::fmt;

use crate::format_version::deserialize_baseline_u32;

use super::Attrs;
use super::naming::Path;
use super::package::{PackageDefinition, PackageSpecification};
use super::types::Type;

/// Distribution of packages
#[derive(Debug, Clone, PartialEq)]
pub struct Distribution {
    pub format_version: u32,
    pub distribution: DistributionBody,
}

impl Distribution {
    /// The `formatVersion` value this distribution writes, chosen by the
    /// content of [`DistributionBody`]: a `Library` writes the classic `3`,
    /// a `Specs` writes `"3.1.0"`, the version that introduced it.
    pub fn emitted_format_version(&self) -> serde_json::Value {
        match &self.distribution {
            DistributionBody::Library(..) => serde_json::Value::from(3u32),
            DistributionBody::Specs(..) => serde_json::Value::from("3.1.0"),
        }
    }
}

impl Serialize for Distribution {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("Distribution", 2)?;
        state.serialize_field("formatVersion", &self.emitted_format_version())?;
        state.serialize_field("distribution", &self.distribution)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for Distribution {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct DistributionFields {
            #[serde(deserialize_with = "deserialize_baseline_u32")]
            format_version: u32,
            distribution: DistributionBody,
        }
        let fields = DistributionFields::deserialize(deserializer)?;
        Ok(Self {
            format_version: fields.format_version,
            distribution: fields.distribution,
        })
    }
}

/// Distribution body - serialized as ["Library", packagePath, dependencies, package]
#[derive(Debug, Clone, PartialEq)]
pub enum DistributionBody {
    Library(
        Path,
        Vec<(Path, PackageSpecification<Attrs>)>,
        PackageDefinition<Attrs, Type<Attrs>>,
    ),
    /// A package's public interface without definitions, introduced in
    /// format version 3.1.0.
    Specs(
        Path,
        Vec<(Path, PackageSpecification<Attrs>)>,
        PackageSpecification<Attrs>,
    ),
}

impl Serialize for DistributionBody {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            DistributionBody::Library(path, deps, package) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("Library")?;
                tuple.serialize_element(path)?;
                tuple.serialize_element(deps)?;
                tuple.serialize_element(package)?;
                tuple.end()
            }
            DistributionBody::Specs(path, deps, spec) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("Specs")?;
                tuple.serialize_element(path)?;
                tuple.serialize_element(deps)?;
                tuple.serialize_element(spec)?;
                tuple.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for DistributionBody {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DistributionBodyVisitor;

        impl<'de> Visitor<'de> for DistributionBodyVisitor {
            type Value = DistributionBody;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str(
                    r#"a DistributionBody array ["Library", path, deps, package] or ["Specs", path, deps, spec]"#,
                )
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let tag: Cow<'de, str> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;

                match tag.as_ref() {
                    "Library" | "library" => {
                        let path = seq
                            .next_element::<Path>()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let deps = seq
                            .next_element::<Vec<(Path, PackageSpecification<Attrs>)>>()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let package = seq
                            .next_element::<PackageDefinition<Attrs, Type<Attrs>>>()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;

                        if let Some(IgnoredAny) = seq.next_element()? {
                            return Err(de::Error::custom(
                                "Expected end of DistributionBody array",
                            ));
                        }

                        Ok(DistributionBody::Library(path, deps, package))
                    }
                    "Specs" | "specs" => {
                        let path = seq
                            .next_element::<Path>()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let deps = seq
                            .next_element::<Vec<(Path, PackageSpecification<Attrs>)>>()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let spec = seq
                            .next_element::<PackageSpecification<Attrs>>()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;

                        if let Some(IgnoredAny) = seq.next_element()? {
                            return Err(de::Error::custom(
                                "Expected end of DistributionBody array",
                            ));
                        }

                        Ok(DistributionBody::Specs(path, deps, spec))
                    }
                    _ => Err(de::Error::unknown_variant(
                        tag.as_ref(),
                        &["Library", "Specs"],
                    )),
                }
            }
        }

        deserializer.deserialize_seq(DistributionBodyVisitor)
    }
}

/// Tag for backward compatibility - no longer needed with custom serde
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LibraryTag {
    #[serde(alias = "library")]
    Library,
}

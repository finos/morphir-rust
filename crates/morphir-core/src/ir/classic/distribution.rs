//! Classic IR Distribution types
//!
//! Distribution wrapper for the Classic Morphir IR format.

use serde::de::{self, IgnoredAny, SeqAccess, Visitor};
use serde::ser::{SerializeTuple, Serializer};
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::fmt;

use super::Attrs;
use super::naming::Path;
use super::package::{PackageDefinition, PackageSpecification};
use super::types::Type;

/// Distribution of packages
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Distribution {
    pub format_version: u32,
    pub distribution: DistributionBody,
}

/// Distribution body - serialized as ["Library", packagePath, dependencies, package]
#[derive(Debug, Clone, PartialEq)]
pub enum DistributionBody {
    Library(
        Path,
        Vec<(Path, PackageSpecification<Attrs>)>,
        PackageDefinition<Attrs, Type<Attrs>>,
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
                formatter.write_str(r#"a DistributionBody array ["Library", path, deps, package]"#)
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
                    _ => Err(de::Error::unknown_variant(tag.as_ref(), &["Library"])),
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

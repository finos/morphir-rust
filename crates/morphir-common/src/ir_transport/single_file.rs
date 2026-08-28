use std::fmt;
use std::io::Read;

use anyhow::{Context, Result, bail};
use morphir_core::ir::classic;
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};

type ClassicDependencies = Vec<(classic::Path, classic::PackageSpecification<classic::Attrs>)>;
type ClassicModule = classic::ModuleEntry<classic::Attrs, classic::Type<classic::Attrs>>;

/// Receives a Classic v3 distribution without retaining its package modules.
pub trait ClassicV3ModuleVisitor {
    type Output;

    fn begin(
        &mut self,
        package: &classic::Path,
        dependencies: &[(classic::Path, classic::PackageSpecification<classic::Attrs>)],
    ) -> std::result::Result<(), String>;

    fn visit_module(&mut self, module: ClassicModule) -> std::result::Result<(), String>;

    fn finish(self) -> std::result::Result<Self::Output, String>;
}

struct DistributionSeed<'visitor, V> {
    visitor: &'visitor mut V,
}

impl<'de, V: ClassicV3ModuleVisitor> DeserializeSeed<'de> for DistributionSeed<'_, V> {
    type Value = u32;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_map(DistributionVisitor {
            visitor: self.visitor,
        })
    }
}

struct DistributionVisitor<'visitor, V> {
    visitor: &'visitor mut V,
}

impl<'de, V: ClassicV3ModuleVisitor> Visitor<'de> for DistributionVisitor<'_, V> {
    type Value = u32;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a Classic Morphir IR distribution object")
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut format_version = None;
        let mut saw_distribution = false;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "formatVersion" => format_version = Some(map.next_value()?),
                "distribution" => {
                    if saw_distribution {
                        return Err(de::Error::duplicate_field("distribution"));
                    }
                    map.next_value_seed(DistributionBodySeed {
                        visitor: self.visitor,
                    })?;
                    saw_distribution = true;
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        if !saw_distribution {
            return Err(de::Error::missing_field("distribution"));
        }
        format_version.ok_or_else(|| de::Error::missing_field("formatVersion"))
    }
}

struct DistributionBodySeed<'visitor, V> {
    visitor: &'visitor mut V,
}

impl<'de, V: ClassicV3ModuleVisitor> DeserializeSeed<'de> for DistributionBodySeed<'_, V> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_seq(DistributionBodyVisitor {
            visitor: self.visitor,
        })
    }
}

struct DistributionBodyVisitor<'visitor, V> {
    visitor: &'visitor mut V,
}

impl<'de, V: ClassicV3ModuleVisitor> Visitor<'de> for DistributionBodyVisitor<'_, V> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(r#"["Library", package, dependencies, definition]"#)
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let tag = sequence
            .next_element::<String>()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        if !tag.eq_ignore_ascii_case("library") {
            return Err(de::Error::unknown_variant(&tag, &["Library"]));
        }
        let package = sequence
            .next_element::<classic::Path>()?
            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
        let dependencies = sequence
            .next_element::<ClassicDependencies>()?
            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
        self.visitor
            .begin(&package, &dependencies)
            .map_err(de::Error::custom)?;
        sequence
            .next_element_seed(PackageSeed {
                visitor: self.visitor,
            })?
            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
        if sequence.next_element::<IgnoredAny>()?.is_some() {
            return Err(de::Error::custom(
                "expected the end of the Classic Library distribution",
            ));
        }
        Ok(())
    }
}

struct PackageSeed<'visitor, V> {
    visitor: &'visitor mut V,
}

impl<'de, V: ClassicV3ModuleVisitor> DeserializeSeed<'de> for PackageSeed<'_, V> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_map(PackageVisitor {
            visitor: self.visitor,
        })
    }
}

struct PackageVisitor<'visitor, V> {
    visitor: &'visitor mut V,
}

impl<'de, V: ClassicV3ModuleVisitor> Visitor<'de> for PackageVisitor<'_, V> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a Classic package definition object")
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut saw_modules = false;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "modules" => {
                    map.next_value_seed(ModulesSeed {
                        visitor: self.visitor,
                    })?;
                    saw_modules = true;
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        if !saw_modules {
            return Err(de::Error::missing_field("modules"));
        }
        Ok(())
    }
}

struct ModulesSeed<'visitor, V> {
    visitor: &'visitor mut V,
}

impl<'de, V: ClassicV3ModuleVisitor> DeserializeSeed<'de> for ModulesSeed<'_, V> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_seq(ModulesVisitor {
            visitor: self.visitor,
        })
    }
}

struct ModulesVisitor<'visitor, V> {
    visitor: &'visitor mut V,
}

impl<'de, V: ClassicV3ModuleVisitor> Visitor<'de> for ModulesVisitor<'_, V> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an array of Classic package modules")
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while let Some(module) = sequence.next_element::<ClassicModule>()? {
            self.visitor
                .visit_module(module)
                .map_err(de::Error::custom)?;
        }
        Ok(())
    }
}

/// Parse a Classic v3 single-file distribution and release each module after visiting it.
pub fn visit_classic_v3<R, V>(reader: R, mut visitor: V) -> Result<V::Output>
where
    R: Read,
    V: ClassicV3ModuleVisitor,
{
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let version = DistributionSeed {
        visitor: &mut visitor,
    }
    .deserialize(&mut deserializer)
    .context("failed to stream Classic IR")?;
    deserializer
        .end()
        .context("unexpected data after Classic IR distribution")?;
    if version != 3 {
        bail!("typed Classic migration requires formatVersion 3, found {version}");
    }
    visitor.finish().map_err(anyhow::Error::msg)
}

use std::fmt;
use std::io::{Read, Seek, SeekFrom};

use anyhow::{Context, Result, bail};
use morphir_core::format_version::FormatVersionBaselineSeed;
use morphir_core::ir::classic;
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};

use super::IR_RECURSION_STACK_BYTES;

type ClassicDependencies = Vec<(classic::Path, classic::PackageSpecification<classic::Attrs>)>;
type ClassicModule = classic::ModuleEntry<classic::Attrs, classic::Type<classic::Attrs>>;
type ClassicModuleSpecification = classic::package::ModuleSpecEntry<classic::Attrs>;

/// Receives a Classic v3 distribution without retaining its package modules.
///
/// A `Library` distribution calls [`begin`](Self::begin) and then
/// [`visit_module`](Self::visit_module) per module; a `Specs` distribution (format version
/// 3.1.0) calls [`begin_specs`](Self::begin_specs) and then
/// [`visit_module_specification`](Self::visit_module_specification) per module. A visitor that
/// does not override the `Specs` methods refuses a `Specs` distribution.
pub trait ClassicV3ModuleVisitor {
    type Output;

    fn begin(
        &mut self,
        package: &classic::Path,
        dependencies: &[(classic::Path, classic::PackageSpecification<classic::Attrs>)],
    ) -> std::result::Result<(), String>;

    fn visit_module(&mut self, module: ClassicModule) -> std::result::Result<(), String>;

    fn begin_specs(
        &mut self,
        _package: &classic::Path,
        _dependencies: &[(classic::Path, classic::PackageSpecification<classic::Attrs>)],
    ) -> std::result::Result<(), String> {
        Err("this Classic v3 visitor does not accept a Specs distribution".to_owned())
    }

    fn visit_module_specification(
        &mut self,
        _module: ClassicModuleSpecification,
    ) -> std::result::Result<(), String> {
        Err("this Classic v3 visitor does not accept a Specs distribution".to_owned())
    }

    fn finish(self) -> std::result::Result<Self::Output, String>;
}

struct DistributionSeed<'visitor, V> {
    visitor: &'visitor mut V,
    prevalidated_version: Option<u32>,
}

impl<'de, V: ClassicV3ModuleVisitor> DeserializeSeed<'de> for DistributionSeed<'_, V> {
    type Value = u32;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_map(DistributionVisitor {
            visitor: self.visitor,
            prevalidated_version: self.prevalidated_version,
        })
    }
}

struct DistributionVisitor<'visitor, V> {
    visitor: &'visitor mut V,
    prevalidated_version: Option<u32>,
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
                "formatVersion" => {
                    if format_version.is_some() {
                        return Err(de::Error::duplicate_field("formatVersion"));
                    }
                    format_version = Some(map.next_value_seed(FormatVersionBaselineSeed)?);
                }
                "distribution" => {
                    if saw_distribution {
                        return Err(de::Error::duplicate_field("distribution"));
                    }
                    match format_version.or(self.prevalidated_version) {
                        Some(3) => {}
                        Some(version) => {
                            return Err(de::Error::custom(format!(
                                "typed Classic migration requires formatVersion 3, found {version}"
                            )));
                        }
                        None => {
                            return Err(de::Error::custom(
                                "formatVersion must precede distribution for streaming decode",
                            ));
                        }
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
        let format_version =
            format_version.ok_or_else(|| de::Error::missing_field("formatVersion"))?;
        if format_version != 3 {
            return Err(de::Error::custom(format!(
                "typed Classic migration requires formatVersion 3, found {format_version}"
            )));
        }
        Ok(format_version)
    }
}

/// Decode Classic v3 modules from any Serde deserializer.
///
/// The deserializer must present `formatVersion` before `distribution`, allowing
/// the visitor to reject non-v3 input before invoking callbacks.
pub fn visit_classic_v3_deserializer<'de, D, V>(
    deserializer: D,
    mut visitor: V,
) -> std::result::Result<V::Output, String>
where
    D: de::Deserializer<'de>,
    V: ClassicV3ModuleVisitor,
{
    deserialize_classic_v3(deserializer, &mut visitor, None).map_err(|error| error.to_string())?;
    visitor.finish()
}

pub(crate) fn deserialize_classic_v3<'de, D, V>(
    deserializer: D,
    visitor: &mut V,
    prevalidated_version: Option<u32>,
) -> std::result::Result<u32, D::Error>
where
    D: de::Deserializer<'de>,
    V: ClassicV3ModuleVisitor,
{
    DistributionSeed {
        visitor,
        prevalidated_version,
    }
    .deserialize(deserializer)
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
        formatter.write_str(
            r#"["Library", package, dependencies, definition] or ["Specs", package, dependencies, specification]"#,
        )
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let tag = sequence
            .next_element::<String>()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let kind = if tag.eq_ignore_ascii_case("library") {
            BodyKind::Library
        } else if tag.eq_ignore_ascii_case("specs") {
            BodyKind::Specs
        } else {
            return Err(de::Error::unknown_variant(&tag, &["Library", "Specs"]));
        };
        let package = sequence
            .next_element::<classic::Path>()?
            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
        let dependencies = sequence
            .next_element::<ClassicDependencies>()?
            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
        match kind {
            BodyKind::Library => self.visitor.begin(&package, &dependencies),
            BodyKind::Specs => self.visitor.begin_specs(&package, &dependencies),
        }
        .map_err(de::Error::custom)?;
        sequence
            .next_element_seed(PackageSeed {
                visitor: self.visitor,
                kind,
            })?
            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
        if sequence.next_element::<IgnoredAny>()?.is_some() {
            return Err(de::Error::custom(format!(
                "expected the end of the Classic {} distribution",
                kind.tag()
            )));
        }
        Ok(())
    }
}

/// Which module kind the fourth element of a Classic distribution body holds.
#[derive(Clone, Copy)]
enum BodyKind {
    /// `["Library", …]`: module definitions.
    Library,
    /// `["Specs", …]`: module specifications.
    Specs,
}

impl BodyKind {
    fn tag(self) -> &'static str {
        match self {
            Self::Library => "Library",
            Self::Specs => "Specs",
        }
    }
}

struct PackageSeed<'visitor, V> {
    visitor: &'visitor mut V,
    kind: BodyKind,
}

impl<'de, V: ClassicV3ModuleVisitor> DeserializeSeed<'de> for PackageSeed<'_, V> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_map(PackageVisitor {
            visitor: self.visitor,
            kind: self.kind,
        })
    }
}

struct PackageVisitor<'visitor, V> {
    visitor: &'visitor mut V,
    kind: BodyKind,
}

impl<'de, V: ClassicV3ModuleVisitor> Visitor<'de> for PackageVisitor<'_, V> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            BodyKind::Library => formatter.write_str("a Classic package definition object"),
            BodyKind::Specs => formatter.write_str("a Classic package specification object"),
        }
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
                        kind: self.kind,
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
    kind: BodyKind,
}

impl<'de, V: ClassicV3ModuleVisitor> DeserializeSeed<'de> for ModulesSeed<'_, V> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_seq(ModulesVisitor {
            visitor: self.visitor,
            kind: self.kind,
        })
    }
}

struct ModulesVisitor<'visitor, V> {
    visitor: &'visitor mut V,
    kind: BodyKind,
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
        loop {
            let visited = stacker::grow(IR_RECURSION_STACK_BYTES, || match self.kind {
                BodyKind::Library => {
                    let Some(module) = sequence.next_element::<ClassicModule>()? else {
                        return Ok(false);
                    };
                    self.visitor
                        .visit_module(module)
                        .map_err(de::Error::custom)?;
                    Ok(true)
                }
                BodyKind::Specs => {
                    // Strict: a module definition under a Specs tag is refused, not read as an
                    // empty specification.
                    let Some(module) = sequence
                        .next_element::<classic::package::SpecsModuleEntry<classic::Attrs>>()?
                    else {
                        return Ok(false);
                    };
                    self.visitor
                        .visit_module_specification(module.into())
                        .map_err(de::Error::custom)?;
                    Ok(true)
                }
            })?;
            if !visited {
                break;
            }
        }
        Ok(())
    }
}

/// Parse a Classic v3 single-file distribution and release each module after visiting it.
pub fn visit_classic_v3<R, V>(mut reader: R, mut visitor: V) -> Result<V::Output>
where
    R: Read,
    V: ClassicV3ModuleVisitor,
{
    use morphir_core::format_version::SupportTable;

    use super::root_probe::{ProbedJsonReader, probe_json_root};

    let (probe, input) = probe_json_root(&mut reader, &SupportTable::reference())
        .map_err(|diagnostic| anyhow::Error::msg(diagnostic.to_string()))?;
    if probe.normalized.release.major() != 3 {
        bail!(
            "typed Classic migration requires formatVersion 3, found {}",
            probe.normalized.release
        );
    }

    let prevalidated = Some(probe.normalized.release.major());
    let version = match input {
        ProbedJsonReader::Stream(mut prefixed) => {
            let mut deserializer = serde_json::Deserializer::from_reader(&mut prefixed);
            let version = DistributionSeed {
                visitor: &mut visitor,
                prevalidated_version: prevalidated,
            }
            .deserialize(&mut deserializer)
            .context("failed to stream Classic IR")?;
            deserializer
                .end()
                .context("unexpected data after Classic IR distribution")?;
            version
        }
        ProbedJsonReader::Memory(mut cursor) => {
            cursor
                .seek(SeekFrom::Start(0))
                .context("failed to rewind Classic IR after reading its format version")?;
            let mut deserializer = serde_json::Deserializer::from_reader(&mut cursor);
            let version = DistributionSeed {
                visitor: &mut visitor,
                prevalidated_version: prevalidated,
            }
            .deserialize(&mut deserializer)
            .context("failed to stream Classic IR")?;
            deserializer
                .end()
                .context("unexpected data after Classic IR distribution")?;
            version
        }
        ProbedJsonReader::Temporary(mut file) => {
            file.seek(SeekFrom::Start(0))
                .context("failed to rewind Classic IR after reading its format version")?;
            let mut deserializer = serde_json::Deserializer::from_reader(&mut file);
            let version = DistributionSeed {
                visitor: &mut visitor,
                prevalidated_version: prevalidated,
            }
            .deserialize(&mut deserializer)
            .context("failed to stream Classic IR")?;
            deserializer
                .end()
                .context("unexpected data after Classic IR distribution")?;
            version
        }
    };
    debug_assert_eq!(version, 3);
    visitor.finish().map_err(anyhow::Error::msg)
}

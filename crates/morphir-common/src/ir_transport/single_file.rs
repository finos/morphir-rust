use std::fmt;
use std::io::{Read, Seek, SeekFrom};

use anyhow::{Context, Result, bail};
use morphir_core::ir::classic;
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};

use super::IR_RECURSION_STACK_BYTES;

type ClassicDependencies = Vec<(classic::Path, classic::PackageSpecification<classic::Attrs>)>;
type ClassicModule = classic::ModuleEntry<classic::Attrs, classic::Type<classic::Attrs>>;

fn read_format_version(reader: &mut impl Read) -> Result<u32> {
    let mut buffer = [0_u8; 1024];
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut capturing_key = false;
    let mut expecting_key = false;
    let mut key = Vec::new();
    let mut pending_key = None;
    let mut reading_version = false;
    let mut version = Vec::new();

    loop {
        let read = reader
            .read(&mut buffer)
            .context("failed to scan Classic IR format version")?;
        if read == 0 {
            bail!("Classic IR is missing formatVersion");
        }
        for byte in &buffer[..read] {
            if in_string {
                if escaped {
                    escaped = false;
                } else if *byte == b'\\' {
                    escaped = true;
                } else if *byte == b'"' {
                    in_string = false;
                    if capturing_key {
                        pending_key = Some(std::mem::take(&mut key));
                        capturing_key = false;
                    }
                } else if capturing_key {
                    key.push(*byte);
                }
                continue;
            }

            if reading_version {
                if byte.is_ascii_digit() {
                    version.push(*byte);
                    continue;
                }
                if byte.is_ascii_whitespace() && version.is_empty() {
                    continue;
                }
                if !version.is_empty()
                    && (byte.is_ascii_whitespace() || matches!(byte, b',' | b'}'))
                {
                    let source = std::str::from_utf8(&version)
                        .context("formatVersion is not valid UTF-8")?;
                    return source
                        .parse()
                        .context("formatVersion must be an unsigned integer");
                }
                bail!("formatVersion must be an unsigned integer");
            }

            match *byte {
                b'"' => {
                    in_string = true;
                    if depth == 1 && expecting_key {
                        capturing_key = true;
                        expecting_key = false;
                    }
                }
                b'{' | b'[' => {
                    depth += 1;
                    if depth == 1 {
                        expecting_key = true;
                    }
                }
                b'}' | b']' => depth = depth.saturating_sub(1),
                b':' if depth == 1 => {
                    if pending_key.as_deref() == Some(b"formatVersion") {
                        reading_version = true;
                    }
                    pending_key = None;
                }
                b',' if depth == 1 => {
                    expecting_key = true;
                    pending_key = None;
                }
                _ => {}
            }
        }
    }
}

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
                    format_version = Some(map.next_value()?);
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
    deserialize_classic_v3(deserializer, &mut visitor).map_err(|error| error.to_string())?;
    visitor.finish()
}

pub(crate) fn deserialize_classic_v3<'de, D, V>(
    deserializer: D,
    visitor: &mut V,
) -> std::result::Result<u32, D::Error>
where
    D: de::Deserializer<'de>,
    V: ClassicV3ModuleVisitor,
{
    DistributionSeed {
        visitor,
        prevalidated_version: None,
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
        loop {
            let visited = stacker::grow(IR_RECURSION_STACK_BYTES, || {
                let Some(module) = sequence.next_element::<ClassicModule>()? else {
                    return Ok(false);
                };
                self.visitor
                    .visit_module(module)
                    .map_err(de::Error::custom)?;
                Ok(true)
            })?;
            if !visited {
                break;
            }
        }
        Ok(())
    }
}

/// Parse a Classic v3 single-file distribution and release each module after visiting it.
pub fn visit_classic_v3<R, V>(reader: R, mut visitor: V) -> Result<V::Output>
where
    R: Read + Seek,
    V: ClassicV3ModuleVisitor,
{
    let mut reader = reader;
    let version = read_format_version(&mut reader)?;
    if version != 3 {
        bail!("typed Classic migration requires formatVersion 3, found {version}");
    }
    reader
        .seek(SeekFrom::Start(0))
        .context("failed to rewind Classic IR after reading its format version")?;
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let version = DistributionSeed {
        visitor: &mut visitor,
        prevalidated_version: Some(version),
    }
    .deserialize(&mut deserializer)
    .context("failed to stream Classic IR")?;
    deserializer
        .end()
        .context("unexpected data after Classic IR distribution")?;
    debug_assert_eq!(version, 3);
    visitor.finish().map_err(anyhow::Error::msg)
}

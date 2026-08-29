//! Serde helpers for wire `formatVersion` scalars.

use std::fmt;

use serde::de::{self, DeserializeSeed, Deserializer, Visitor};

use super::{
    CanonicalSpelling, FormatVersionDiagnostic, NormalizedFormatVersion, ScalarValue, SupportTable,
};

/// Deserialize a normalized baseline `formatVersion` major as `u32`.
pub fn deserialize_baseline_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    FormatVersionBaselineSeed.deserialize(deserializer)
}

/// Seed that deserializes one wire scalar into its canonical baseline integer.
pub struct FormatVersionBaselineSeed;

impl<'de> DeserializeSeed<'de> for FormatVersionBaselineSeed {
    type Value = u32;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(FormatVersionBaselineVisitor)
    }
}

struct FormatVersionBaselineVisitor;

impl<'de> Visitor<'de> for FormatVersionBaselineVisitor {
    type Value = u32;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a supported formatVersion string or unsigned integer")
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        scalar_to_baseline_u32(ScalarValue::Integer(value)).map_err(de::Error::custom)
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value < 0 {
            return Err(de::Error::custom(
                FormatVersionDiagnostic::invalid_format_version_type().to_string(),
            ));
        }
        self.visit_u64(value as u64)
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        scalar_to_baseline_u32(ScalarValue::String(value.to_owned())).map_err(de::Error::custom)
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(&value)
    }
}

fn scalar_to_baseline_u32(scalar: ScalarValue) -> Result<u32, FormatVersionDiagnostic> {
    let normalized = NormalizedFormatVersion::from_scalar(&scalar, &SupportTable::reference())?;
    match normalized.canonical {
        CanonicalSpelling::Integer(version) => Ok(version),
        CanonicalSpelling::String(release) => Err(FormatVersionDiagnostic::new(
            "unsupported_format_version_revision",
            format!("release {release} is recognized but not supported as a baseline integer"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct VersionField {
        #[serde(
            rename = "formatVersion",
            deserialize_with = "deserialize_baseline_u32"
        )]
        format_version: u32,
    }

    #[test]
    fn string_three_zero_zero_deserializes_to_baseline_three() {
        let value: VersionField =
            serde_json::from_str(r#"{"formatVersion":"3.0.0"}"#).expect("baseline string");
        assert_eq!(value.format_version, 3);
    }
}

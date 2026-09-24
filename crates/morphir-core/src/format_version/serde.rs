//! Serde helpers for wire `formatVersion` scalars.

use std::fmt;

use serde::de::{self, DeserializeSeed, Deserializer, Visitor};

use super::{
    FormatVersionDiagnostic, NormalizedFormatVersion, ReleaseTriplet, ScalarValue, SupportTable,
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
        FormatVersionReleaseSeed
            .deserialize(deserializer)
            .map(|declared| declared.release.major())
    }
}

/// A supported wire `formatVersion`: its normalized release and the spelling the document used.
///
/// A reader keyed by major (the classic model) keeps the release as well when a rule depends on
/// the minor version, such as a v3 `Specs` distribution needing 3.1.0 or later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredRelease {
    /// The exact release after normalization: integer `3` is `3.0.0`.
    pub release: ReleaseTriplet,
    /// The value as the document wrote it: `3` or `3.0.0`.
    pub declared: String,
}

impl DeclaredRelease {
    /// A release known without its written spelling, for example from a root probe.
    pub fn from_release(release: ReleaseTriplet) -> Self {
        Self {
            declared: release.to_string(),
            release,
        }
    }
}

/// Deserialize a supported `formatVersion` as its release and declared spelling.
pub fn deserialize_declared_release<'de, D>(deserializer: D) -> Result<DeclaredRelease, D::Error>
where
    D: Deserializer<'de>,
{
    FormatVersionReleaseSeed.deserialize(deserializer)
}

/// Seed that deserializes one supported wire scalar into a [`DeclaredRelease`].
pub struct FormatVersionReleaseSeed;

impl<'de> DeserializeSeed<'de> for FormatVersionReleaseSeed {
    type Value = DeclaredRelease;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(FormatVersionReleaseVisitor)
    }
}

struct FormatVersionReleaseVisitor;

impl<'de> Visitor<'de> for FormatVersionReleaseVisitor {
    type Value = DeclaredRelease;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a supported formatVersion string or unsigned integer")
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        declared_release(ScalarValue::Integer(value)).map_err(de::Error::custom)
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
        declared_release(ScalarValue::String(value.to_owned())).map_err(de::Error::custom)
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(&value)
    }
}

fn declared_release(scalar: ScalarValue) -> Result<DeclaredRelease, FormatVersionDiagnostic> {
    let declared = match &scalar {
        ScalarValue::Integer(value) => value.to_string(),
        ScalarValue::String(value) => value.clone(),
    };
    Ok(DeclaredRelease {
        release: supported_release(scalar)?,
        declared,
    })
}

#[cfg(test)]
fn scalar_to_baseline_u32(scalar: ScalarValue) -> Result<u32, FormatVersionDiagnostic> {
    supported_release(scalar).map(|release| release.major())
}

fn supported_release(scalar: ScalarValue) -> Result<ReleaseTriplet, FormatVersionDiagnostic> {
    let support = SupportTable::reference();
    let normalized = NormalizedFormatVersion::from_scalar(&scalar, &support)?;
    // The classic model is keyed by major, and the patch promise says a reader
    // that understands `N.m.0` reads every `N.m.p`. So compatibility against the
    // table decides, not the canonical spelling: a supported later patch such as
    // `"3.0.1"` still maps to its major.
    if let Some(diagnostic) =
        support.unsupported_diagnostic(&normalized.release, normalized.compatibility)
    {
        return Err(diagnostic);
    }
    Ok(normalized.release)
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

    fn baseline(value: &str) -> Result<u32, String> {
        scalar_to_baseline_u32(ScalarValue::String(value.to_owned()))
            .map_err(|diagnostic| diagnostic.code().to_string())
    }

    #[test]
    fn supported_later_patches_map_to_their_major() {
        assert_eq!(baseline("3.0.1"), Ok(3));
        assert_eq!(baseline("4.0.1"), Ok(4));
    }

    #[test]
    fn unsupported_minor_is_reported_as_a_minor() {
        assert_eq!(
            baseline("3.2.0"),
            Err("unsupported_format_version_minor".to_owned())
        );
    }

    #[test]
    fn unsupported_major_is_reported_as_a_major() {
        assert_eq!(
            baseline("5.0.0"),
            Err("unsupported_format_version_major".to_owned())
        );
    }

    #[test]
    fn integer_four_deserializes_to_baseline_four() {
        assert_eq!(
            scalar_to_baseline_u32(ScalarValue::Integer(4))
                .map_err(|diagnostic| diagnostic.code().to_string()),
            Ok(4)
        );
    }
}

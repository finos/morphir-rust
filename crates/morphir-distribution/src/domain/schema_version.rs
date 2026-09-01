//! Schema version parsing and compatibility.

use crate::error::{Result, invalid_value};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// A major/minor schema version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaVersion {
    major: u32,
    minor: u32,
}

impl SchemaVersion {
    /// Construct a schema version from its major and minor components.
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// Return whether this version supports a candidate version.
    pub const fn supports(self, candidate: Self) -> bool {
        self.major == candidate.major && candidate.minor <= self.minor
    }

    /// Parse a canonical `major.minor` schema version.
    pub fn parse(value: &str) -> Result<Self> {
        let Some((major, minor)) = value.split_once('.') else {
            return Err(invalid_schema_version(value));
        };
        let Some(major) = parse_component(major) else {
            return Err(invalid_schema_version(value));
        };
        let Some(minor) = parse_component(minor) else {
            return Err(invalid_schema_version(value));
        };
        Ok(Self::new(major, minor))
    }
}

fn parse_component(component: &str) -> Option<u32> {
    if component.is_empty()
        || (component.len() > 1 && component.starts_with('0'))
        || !component.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    component.parse().ok()
}

fn invalid_schema_version(value: &str) -> crate::DistributionError {
    invalid_value(
        "schema version",
        value,
        "expected exactly two canonical unsigned decimal components separated by one dot",
    )
}

impl fmt::Display for SchemaVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

impl FromStr for SchemaVersion {
    type Err = crate::DistributionError;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl Serialize for SchemaVersion {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SchemaVersion {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::SchemaVersion;

    #[test]
    fn schema_version_uses_strict_major_minor_strings() {
        let version: SchemaVersion = serde_json::from_str(r#""1.2""#).unwrap();
        assert_eq!(version, SchemaVersion::new(1, 2));
        assert_eq!(serde_json::to_string(&version).unwrap(), r#""1.2""#);

        for invalid in [r#"1"#, r#""1""#, r#""1.2.0""#, r#""01.2""#, r#""1.02""#] {
            assert!(
                serde_json::from_str::<SchemaVersion>(invalid).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn schema_version_supports_older_minors_of_the_same_major() {
        let current = SchemaVersion::new(1, 2);
        assert!(current.supports(SchemaVersion::new(1, 0)));
        assert!(current.supports(SchemaVersion::new(1, 2)));
        assert!(!current.supports(SchemaVersion::new(1, 3)));
        assert!(!current.supports(SchemaVersion::new(2, 0)));
    }
}

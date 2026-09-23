//! Extension schema versions, retaining the released wire spelling.

use crate::SchemaVersion;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::fmt;

/// An extension schema version with legacy `1.0` compatibility.
/// Stable major 1 and exactly `2.0.0-draft.1` are supported. No major 0
/// schema was released; when major 2 releases, major 1 remains readable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ExtensionSchemaVersion {
    /// The original two-component spelling.
    Legacy(SchemaVersion),
    /// A SemVer schema identifier.
    Semver(Version),
}

impl Default for ExtensionSchemaVersion {
    fn default() -> Self {
        Self::Legacy(SchemaVersion::new(1, 0))
    }
}

impl fmt::Display for ExtensionSchemaVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Legacy(version) => version.fmt(f),
            Self::Semver(version) => version.fmt(f),
        }
    }
}

impl PartialEq<SchemaVersion> for ExtensionSchemaVersion {
    fn eq(&self, other: &SchemaVersion) -> bool {
        matches!(self, Self::Legacy(version) if version == other)
    }
}

impl<'de> Deserialize<'de> for ExtensionSchemaVersion {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if let Ok(legacy) = SchemaVersion::parse(&text) {
            return if legacy == SchemaVersion::new(1, 0) {
                Ok(Self::Legacy(legacy))
            } else {
                Err(serde::de::Error::custom(format!(
                    "unsupported extension schema version {text}; legacy supported range is 1.0 through 1.0"
                )))
            };
        }
        let version = Version::parse(&text).map_err(serde::de::Error::custom)?;
        let draft = Version::parse("2.0.0-draft.1").expect("supported draft is SemVer");
        if (version.pre.is_empty() && version.major == 1) || version.cmp_precedence(&draft).is_eq()
        {
            Ok(Self::Semver(version))
        } else {
            Err(serde::de::Error::custom(format!(
                "unsupported extension schema version {text}; supported released major: 1; exact drafts: 2.0.0-draft.1"
            )))
        }
    }
}

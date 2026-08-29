//! Portable identities, channels, filenames, and relative paths.

use crate::error::{Result, invalid_value};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::fmt;
use std::path::{Component, Path};

pub(crate) fn portable_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && value.as_bytes()[0].is_ascii_lowercase()
        && value.as_bytes()[value.len() - 1].is_ascii_alphanumeric()
}

/// A lowercase portable extension identifier such as `morphir-elm`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExtensionId(String);

impl ExtensionId {
    /// Parse and validate an extension identifier.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if portable_token(&value) {
            Ok(Self(value))
        } else {
            Err(invalid_value(
                "extension id",
                value,
                "expected a lowercase portable token beginning with a letter",
            ))
        }
    }

    /// Return the portable identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExtensionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for ExtensionId {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ExtensionId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// A lowercase portable tool identifier such as `desktop`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ToolId(String);

impl ToolId {
    /// Parse and validate a tool identifier.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if portable_token(&value) {
            Ok(Self(value))
        } else {
            Err(invalid_value(
                "tool id",
                value,
                "expected a lowercase portable token",
            ))
        }
    }

    /// Return the portable identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ToolId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for ToolId {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ToolId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// A validated segment in a named preview channel.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChannelSegment(String);

impl ChannelSegment {
    /// Parse a lowercase channel segment.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if portable_token(&value) {
            Ok(Self(value))
        } else {
            Err(invalid_value(
                "preview channel segment",
                value,
                "expected a lowercase portable token",
            ))
        }
    }

    /// Return the channel segment.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A moving extension release channel.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Channel {
    /// Production releases. Prerelease semantic versions never match.
    Stable,
    /// All preview releases, or one named preview stream.
    Preview(Option<ChannelSegment>),
    /// Alias for the whole preview family whose spelling is retained in locks.
    Insiders,
}

impl Channel {
    /// Parse one supported channel spelling.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "stable" => Ok(Self::Stable),
            "preview" => Ok(Self::Preview(None)),
            "insiders" => Ok(Self::Insiders),
            _ => value
                .strip_prefix("preview/")
                .map(ChannelSegment::parse)
                .transpose()?
                .map(|segment| Self::Preview(Some(segment)))
                .ok_or_else(|| {
                    invalid_value(
                        "channel",
                        value,
                        "expected stable, preview, insiders, or preview/<segment>",
                    )
                }),
        }
    }

    /// Return the exact channel spelling represented by this value.
    pub fn as_str(&self) -> Cow<'_, str> {
        match self {
            Self::Stable => Cow::Borrowed("stable"),
            Self::Preview(None) => Cow::Borrowed("preview"),
            Self::Preview(Some(segment)) => Cow::Owned(format!("preview/{}", segment.as_str())),
            Self::Insiders => Cow::Borrowed("insiders"),
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.as_str())
    }
}

impl Serialize for Channel {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.as_str())
    }
}

impl<'de> Deserialize<'de> for Channel {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// One portable filename used in the content-addressed store.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactFilename(String);

impl ArtifactFilename {
    /// Parse a filename valid as one component on Unix and Windows.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let windows_stem = value
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let windows_reserved = matches!(windows_stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || windows_stem
                .strip_prefix("COM")
                .is_some_and(is_windows_device_number)
            || windows_stem
                .strip_prefix("LPT")
                .is_some_and(is_windows_device_number);
        let invalid = value.is_empty()
            || value == "."
            || value == ".."
            || value.ends_with(['.', ' '])
            || value.bytes().any(|byte| {
                byte < 32
                    || matches!(
                        byte,
                        b'<' | b'>' | b':' | b'"' | b'/' | b'\\' | b'|' | b'?' | b'*'
                    )
            })
            || windows_reserved;
        if invalid {
            Err(invalid_value(
                "artifact filename",
                value,
                "expected one portable filename component",
            ))
        } else {
            Ok(Self(value))
        }
    }

    /// Return the filename.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_windows_device_number(number: &str) -> bool {
    number.len() == 1 && matches!(number.as_bytes()[0], b'1'..=b'9')
}

impl Serialize for ArtifactFilename {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ArtifactFilename {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// A normalized UTF-8 relative path declared by a local-file source or catalog.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelativeArtifactPath(String);

impl RelativeArtifactPath {
    /// Parse a forward-slash relative path without empty or special segments.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let valid = !value.is_empty()
            && !value.starts_with('/')
            && !value.contains(['\\', ':', '\0'])
            && !value.bytes().any(|byte| byte < 32)
            && value
                .split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
        if valid {
            Ok(Self(value))
        } else {
            Err(invalid_value(
                "local artifact path",
                value,
                "expected a normalized UTF-8 relative path with forward slashes",
            ))
        }
    }

    /// Convert a native relative path into its portable forward-slash spelling.
    pub fn from_native_path(path: &Path) -> Result<Self> {
        let segments = path
            .components()
            .map(|component| match component {
                Component::Normal(segment) => segment.to_str().ok_or_else(|| {
                    invalid_value(
                        "local artifact path",
                        path.to_string_lossy(),
                        "expected a UTF-8 relative path",
                    )
                }),
                _ => Err(invalid_value(
                    "local artifact path",
                    path.to_string_lossy(),
                    "expected a relative path without special segments",
                )),
            })
            .collect::<Result<Vec<_>>>()?;
        Self::parse(segments.join("/"))
    }

    /// Return the portable path spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Return the relative path for local filesystem access.
    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl Serialize for RelativeArtifactPath {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RelativeArtifactPath {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::RelativeArtifactPath;
    use std::path::PathBuf;

    #[test]
    fn native_relative_paths_are_stored_with_portable_separators() {
        let native = PathBuf::from("store")
            .join("extensions")
            .join("sha256")
            .join("digest")
            .join("example");

        let path = RelativeArtifactPath::from_native_path(&native).unwrap();

        assert_eq!(path.as_str(), "store/extensions/sha256/digest/example");
    }
}

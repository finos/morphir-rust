use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::diagnostic::WORKSPACE_PATH_NOT_CONFINED;

/// A canonical slash-separated path confined beneath a named mount.
///
/// The value `.` denotes the mount root. All other values contain only
/// non-empty path components and cannot contain `.` or `..` components.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct RelativePath(String);

impl RelativePath {
    /// Parses a canonical path relative to a named mount.
    pub fn parse(path: impl Into<String>) -> Result<Self, RelativePathError> {
        let path = path.into();

        if is_confined(&path) {
            Ok(Self(path))
        } else {
            Err(RelativePathError::NotConfined { path })
        }
    }

    /// Returns the canonical mount root.
    #[must_use]
    pub fn root() -> Self {
        Self(".".to_owned())
    }

    /// Joins another canonical relative path without normalizing escapes.
    pub fn join(&self, path: impl AsRef<str>) -> Result<Self, RelativePathError> {
        let path = Self::parse(path.as_ref())?;

        match (self.as_str(), path.as_str()) {
            (".", _) => Ok(path),
            (_, ".") => Ok(self.clone()),
            (base, child) => Self::parse(format!("{base}/{child}")),
        }
    }

    /// Returns the canonical parent, keeping the mount root at `.`.
    #[must_use]
    pub fn parent(&self) -> Self {
        match self.0.rsplit_once('/') {
            Some((parent, _)) => Self(parent.to_owned()),
            None => Self::root(),
        }
    }

    /// Returns this path's canonical wire representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for RelativePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let path = String::deserialize(deserializer)?;
        Self::parse(path).map_err(serde::de::Error::custom)
    }
}

/// An error returned when a path is not confined to its named mount.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RelativePathError {
    /// The supplied path was not a canonical, confined relative path.
    #[error("{WORKSPACE_PATH_NOT_CONFINED}: path `{path}` is not confined to its mount")]
    NotConfined {
        /// The rejected path.
        path: String,
    },
}

fn is_confined(path: &str) -> bool {
    if path == "." {
        return true;
    }

    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || has_windows_drive_prefix(path)
    {
        return false;
    }

    path.split('/')
        .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

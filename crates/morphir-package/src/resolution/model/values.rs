use serde::{Serialize, Serializer};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

/// Why a primitive value could not cross the resolution domain boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ResolutionValueError {
    /// The value is not an authority-bearing package path.
    #[error("invalid package path")]
    PackagePath,
    /// The value is not a stable `major.minor.patch` version.
    #[error("invalid stable version")]
    StableVersion,
    /// The value is not an unversioned Morphir IR package name.
    #[error("invalid IR package name")]
    IrPackageName,
    /// The value is not a lowercase SHA-256 digest.
    #[error("invalid SHA-256 digest")]
    ResolutionDigest,
}

/// An authority-bearing package path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct PackagePath(String);

impl PackagePath {
    /// Parse and validate an authority-bearing package path.
    pub fn parse(value: &str) -> Result<Self, ResolutionValueError> {
        valid_package_path(value)
            .then(|| Self(value.to_owned()))
            .ok_or(ResolutionValueError::PackagePath)
    }
    /// The validated wire spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackagePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A stable `major.minor.patch` version with unbounded decimal components.
#[derive(Debug, Clone, Eq)]
pub struct StableVersion {
    text: String,
    components: [String; 3],
}

impl StableVersion {
    /// Parse a stable version without bounding its decimal components.
    pub fn parse(value: &str) -> Result<Self, ResolutionValueError> {
        if !valid_version(value) {
            return Err(ResolutionValueError::StableVersion);
        }
        let mut parts = value.split('.');
        let components = [
            parts.next().unwrap_or_default().to_owned(),
            parts.next().unwrap_or_default().to_owned(),
            parts.next().unwrap_or_default().to_owned(),
        ];
        Ok(Self {
            text: value.to_owned(),
            components,
        })
    }
    /// The validated wire spelling.
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl PartialEq for StableVersion {
    fn eq(&self, other: &Self) -> bool {
        self.components == other.components
    }
}
impl Hash for StableVersion {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.components.hash(state);
    }
}
impl PartialOrd for StableVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for StableVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.components
            .iter()
            .zip(&other.components)
            .find_map(|(left, right)| {
                let ordering = left.len().cmp(&right.len()).then_with(|| left.cmp(right));
                ordering.ne(&Ordering::Equal).then_some(ordering)
            })
            .unwrap_or(Ordering::Equal)
    }
}
impl Serialize for StableVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.text)
    }
}

/// An unversioned Morphir IR package name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct IrPackageName(String);
impl IrPackageName {
    /// Parse and validate an unversioned Morphir IR package name.
    pub fn parse(value: &str) -> Result<Self, ResolutionValueError> {
        valid_ir_name(value)
            .then(|| Self(value.to_owned()))
            .ok_or(ResolutionValueError::IrPackageName)
    }
    /// The validated wire spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A syntactically validated SHA-256 digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ResolutionDigest(String);
impl ResolutionDigest {
    /// Parse and validate a lowercase SHA-256 digest.
    pub fn parse(value: &str) -> Result<Self, ResolutionValueError> {
        valid_digest(value)
            .then(|| Self(value.to_owned()))
            .ok_or(ResolutionValueError::ResolutionDigest)
    }
    /// The validated wire spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The exact identity of one Library release.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseId {
    pub(crate) package_path: PackagePath,
    pub(crate) version: StableVersion,
}
impl ReleaseId {
    /// Combine an already validated package path and exact stable version.
    pub fn new(package_path: PackagePath, version: StableVersion) -> Self {
        Self {
            package_path,
            version,
        }
    }

    /// The authority-bearing package path.
    pub fn package_path(&self) -> &PackagePath {
        &self.package_path
    }
    /// The exact stable release version.
    pub fn version(&self) -> &StableVersion {
        &self.version
    }
}

fn valid_package_path(value: &str) -> bool {
    let Some((domain, path)) = value.split_once('/') else {
        return false;
    };
    domain.contains('.')
        && domain.split('.').all(valid_lower_segment)
        && !path.is_empty()
        && path.split('/').all(valid_lower_segment)
}

fn valid_lower_segment(value: &str) -> bool {
    !value.is_empty()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn valid_ir_name(value: &str) -> bool {
    !value.is_empty()
        && value.split('/').all(|segment| {
            !segment.is_empty()
                && segment.split('-').all(|part| {
                    !part.is_empty()
                        && (part
                            .bytes()
                            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                            || part
                                .bytes()
                                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()))
                })
        })
}

fn valid_version(value: &str) -> bool {
    let mut parts = value.split('.');
    let valid_part = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|byte| byte.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'))
    };
    parts.by_ref().take(3).all(valid_part)
        && parts.next().is_none()
        && value.matches('.').count() == 2
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smart_constructors_reject_invalid_primitives() {
        assert_eq!(
            PackagePath::parse("Example.com/pkg"),
            Err(ResolutionValueError::PackagePath)
        );
        assert_eq!(
            StableVersion::parse("01.0.0"),
            Err(ResolutionValueError::StableVersion)
        );
        assert_eq!(
            IrPackageName::parse("example//pkg"),
            Err(ResolutionValueError::IrPackageName)
        );
        assert_eq!(
            ResolutionDigest::parse("sha256:abc"),
            Err(ResolutionValueError::ResolutionDigest)
        );
    }
}

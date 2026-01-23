use super::Path;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::fmt;
use std::str::FromStr;

/// PackageName is a newtype wrapper around Path for type safety.
/// It distinguishes package paths from module paths at the type level.
///
/// Serializes as a canonical string (e.g., "my-org/my-lib") for V4 format.
/// Deserializes from both string (V4) and array (Classic) formats.
#[derive(Debug, Clone, PartialEq, Eq, Hash, JsonSchema)]
pub struct PackageName(pub Path);

impl Serialize for PackageName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as canonical string for V4 format
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for PackageName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Delegate to Path which handles both string and array formats
        let path = Path::deserialize(deserializer)?;
        Ok(PackageName(path))
    }
}

impl PackageName {
    /// Create a new PackageName from a Path
    pub fn new(path: Path) -> Self {
        Self(path)
    }

    /// Create a PackageName from a string (e.g., "org/package")
    ///
    /// This is a convenience wrapper around `FromStr::from_str`.
    pub fn parse(s: &str) -> Self {
        Self(Path::new(s))
    }

    /// Get the underlying Path
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// Convert to the underlying Path
    pub fn into_path(self) -> Path {
        self.0
    }

    /// Check if the package name is empty
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Path> for PackageName {
    fn from(path: Path) -> Self {
        Self(path)
    }
}

impl From<PackageName> for Path {
    fn from(pkg: PackageName) -> Self {
        pkg.0
    }
}

impl FromStr for PackageName {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Path::new(s)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_name_from_str() {
        let pkg = PackageName::parse("org/morphir/sdk");
        assert_eq!(pkg.to_string(), "org/morphir/sdk");
    }

    #[test]
    fn test_package_name_equality() {
        let pkg1 = PackageName::parse("my/package");
        let pkg2 = PackageName::parse("my/package");
        assert_eq!(pkg1, pkg2);
    }

    #[test]
    fn test_package_name_from_str_trait() {
        let pkg: PackageName = "my/package".parse().unwrap();
        assert_eq!(pkg.to_string(), "my/package");
    }

    #[test]
    fn test_package_name_from_path() {
        let path = Path::new("test/pkg");
        let pkg = PackageName::from(path.clone());
        assert_eq!(pkg.as_path(), &path);
    }
}

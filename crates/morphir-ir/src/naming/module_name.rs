use super::Path;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::fmt;
use std::str::FromStr;

/// ModuleName is a newtype wrapper around Path for type safety.
/// It distinguishes module paths from package paths at the type level.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ModuleName(pub Path);

impl ModuleName {
    /// Create a new ModuleName from a Path
    pub fn new(path: Path) -> Self {
        Self(path)
    }

    /// Create a ModuleName from a string (e.g., "Module/SubModule")
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

    /// Check if the module name is empty
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for ModuleName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Path> for ModuleName {
    fn from(path: Path) -> Self {
        Self(path)
    }
}

impl From<ModuleName> for Path {
    fn from(module: ModuleName) -> Self {
        module.0
    }
}

impl FromStr for ModuleName {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Path::new(s)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_name_from_str() {
        let module = ModuleName::parse("Test/Module");
        assert_eq!(module.to_string(), "test/module");
    }

    #[test]
    fn test_module_name_equality() {
        let m1 = ModuleName::parse("my/module");
        let m2 = ModuleName::parse("my/module");
        assert_eq!(m1, m2);
    }

    #[test]
    fn test_module_name_from_str_trait() {
        let module: ModuleName = "Test/Module".parse().unwrap();
        assert_eq!(module.to_string(), "test/module");
    }

    #[test]
    fn test_module_name_from_path() {
        let path = Path::new("test/mod");
        let module = ModuleName::from(path.clone());
        assert_eq!(module.as_path(), &path);
    }
}

use crate::naming::{name::Name, path::Path};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// FQName represents a Fully Qualified Name (PackagePath + ModulePath + LocalName).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub struct FQName {
    pub package_path: Path,
    pub module_path: Path,
    pub local_name: Name,
}

impl FQName {
    pub fn new(package_path: Path, module_path: Path, local_name: Name) -> Self {
        Self {
            package_path,
            module_path,
            local_name,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 3 {
            return None;
        }
        let pkg_params = parts[0];
        let mod_params = parts[1];
        let local_name = parts[2];

        Some(Self::new(
            Path::new(pkg_params),
            Path::new(mod_params),
            Name::from(local_name),
        ))
    }
}

impl std::fmt::Display for FQName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.package_path, self.module_path, self.local_name
        )
    }
}

impl From<FQName> for String {
    fn from(fqname: FQName) -> String {
        fqname.to_string()
    }
}

impl TryFrom<String> for FQName {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        FQName::parse(&s).ok_or_else(|| format!("Invalid FQName string: {}", s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fqname_parsing() {
        let fq = FQName::parse("org/pkg:mod/sub:Func").unwrap();
        assert_eq!(fq.package_path.to_string(), "org/pkg");
        assert_eq!(fq.module_path.to_string(), "mod/sub");
        assert_eq!(fq.local_name.to_kebab_case(), "func");
    }

    #[test]
    fn test_fqname_roundtrip() {
        let fq = FQName::parse("my/pkg:my/mod:myFunc").unwrap();
        let s = fq.to_string();
        assert_eq!(s, "my/pkg:my/mod:my-func");
    }
}

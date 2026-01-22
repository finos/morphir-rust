use crate::naming::{name::Name, path::Path};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// QName represents a Qualified Name (Path + Name).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub struct QName {
    pub module_path: Path,
    pub local_name: Name,
}

impl QName {
    pub fn new(module_path: Path, local_name: Name) -> Self {
        Self {
            module_path,
            local_name,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            return None;
        }
        let path_str = parts[0];
        let name_str = parts[1];
        Some(Self::new(Path::new(path_str), Name::from(name_str)))
    }
}

impl std::fmt::Display for QName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.module_path, self.local_name)
    }
}

impl From<QName> for String {
    fn from(qname: QName) -> String {
        qname.to_string()
    }
}

impl TryFrom<String> for QName {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        QName::parse(&s).ok_or_else(|| format!("Invalid QName string: {}", s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qname_parsing() {
        let q = QName::parse("foo/bar:Baz").unwrap();
        assert_eq!(q.module_path.to_string(), "foo/bar");
        assert_eq!(q.local_name.to_kebab_case(), "baz");
    }

    #[test]
    fn test_qname_roundtrip() {
        let q = QName::parse("mypkg/mymod:MyFunc").unwrap();
        let s = q.to_string();
        assert_eq!(s, "mypkg/mymod:my-func");
    }
}

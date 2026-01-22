use crate::naming::name::Name;
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;
use std::fmt;

/// A Path is a list of Names, representing a hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Hash, JsonSchema)]
pub struct Path {
    pub segments: Vec<Name>,
}

impl Path {
    pub fn new(s: &str) -> Self {
        let segments = s.split('/').map(Name::new).collect();
        Self { segments }
    }

    pub fn from_names(names: Vec<Name>) -> Self {
        Self { segments: names }
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.segments.iter().map(|n| n.to_kebab_case()).collect();
        write!(f, "{}", parts.join("/"))
    }
}

impl Serialize for Path {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let s = self.to_string();
        serializer.serialize_str(&s)
    }
}

impl<'de> Deserialize<'de> for Path {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum PathFormat {
            String(String),
            Array(Vec<Name>),
            Legacy(Vec<Vec<String>>),
        }

        let v = PathFormat::deserialize(deserializer)?;
        match v {
            PathFormat::String(s) => Ok(Path::new(&s)),
            PathFormat::Array(arr) => Ok(Path::from_names(arr)),
            PathFormat::Legacy(parts) => {
                 let names: Vec<Name> = parts.into_iter().map(|seg| Name::new(&seg.join("_"))).collect();
                 Ok(Path::from_names(names))
            }
        }
    }
}

impl From<&str> for Path {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_parsing() {
        let p = Path::new("foo/bar-baz");
        assert_eq!(p.segments.len(), 2);
        assert_eq!(p.segments[0].to_kebab_case(), "foo");
        assert_eq!(p.segments[1].to_kebab_case(), "bar-baz");
    }

    #[test]
    fn test_path_serialization_canonical() {
        let p = Path::new("foo/bar");
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, "\"foo/bar\"");
    }

    #[test]
    fn test_path_deserialization_canonical() {
        let json = "\"foo/bar\"";
        let p: Path = serde_json::from_str(json).unwrap();
        assert_eq!(p.to_string(), "foo/bar");
    }

    #[test]
    fn test_path_deserialization_legacy() {
        let json = "[[\"foo\"], [\"bar\"]]";
        let p: Path = serde_json::from_str(json).unwrap();
        assert_eq!(p.to_string(), "foo/bar");
    }
}

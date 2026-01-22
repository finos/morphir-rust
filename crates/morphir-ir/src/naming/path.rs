use super::Name;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, JsonSchema)]
pub struct Path {
    pub segments: Vec<Name>,
}

impl Path {
    pub fn new(source: &str) -> Self {
        let segments = source.split('/').map(Name::from).collect();
        Path { segments }
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self
            .segments
            .iter()
            .map(|n: &super::Name| n.to_kebab_case())
            .collect();
        write!(f, "{}", parts.join("/"))
    }
}

impl Serialize for Path {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.segments.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Path {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let segments = Vec::<Name>::deserialize(deserializer)?;
        Ok(Path { segments })
    }
}

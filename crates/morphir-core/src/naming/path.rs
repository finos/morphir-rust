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
        let segments = if source.is_empty() {
            Vec::new()
        } else {
            source.split('/').map(Name::from).collect()
        };
        Path { segments }
    }

    pub fn from_canonical_string(source: &str) -> Result<Self, String> {
        if source.is_empty() {
            return Ok(Self {
                segments: Vec::new(),
            });
        }

        source
            .split('/')
            .map(Name::from_canonical_string)
            .collect::<Result<Vec<_>, _>>()
            .map(|segments| Self { segments })
    }

    pub fn to_canonical_string(&self) -> String {
        self.segments
            .iter()
            .map(Name::to_canonical_string)
            .collect::<Vec<_>>()
            .join("/")
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_canonical_string())
    }
}

impl Serialize for Path {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_canonical_string())
    }
}

impl<'de> Deserialize<'de> for Path {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de;

        // Accept both array format (Classic) and string format (V4)
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            // V4 canonical string format: "my-org/my-lib" or "test-package"
            serde_json::Value::String(s) => {
                Path::from_canonical_string(&s).map_err(de::Error::custom)
            }
            // Classic array format: [["my"], ["org"], ["my"], ["lib"]]
            serde_json::Value::Array(arr) => {
                let segments: Result<Vec<Name>, _> = arr
                    .into_iter()
                    .map(|v| serde_json::from_value(v).map_err(de::Error::custom))
                    .collect();
                Ok(Path {
                    segments: segments?,
                })
            }
            _ => Err(de::Error::custom("expected string or array for Path")),
        }
    }
}

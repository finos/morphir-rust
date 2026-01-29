//! Classic IR Naming types
//!
//! Name, Path, and FQName for the Classic Morphir IR format.
//! These use array-based serialization instead of string-based.

use serde::de::{self, SeqAccess, Visitor};
use serde::ser::{SerializeTuple, Serializer};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

/// Classic Name - a list of words, serialized as ["word1", "word2"]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Name {
    pub words: Vec<String>,
}

impl Name {
    pub fn new(words: Vec<String>) -> Self {
        Self { words }
    }

    pub fn from_str(s: &str) -> Self {
        // Implements behavior matching Elm's regex: "([a-zA-Z][a-z]*|[0-9]+)"
        let mut words = Vec::new();
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        let len = chars.len();

        while i < len {
            let c = chars[i];
            
            if c.is_ascii_alphabetic() {
                // Match [a-zA-Z]
                // Start of a word
                let start = i;
                i += 1;
                // Match [a-z]* (zero or more lowercase letters)
                while i < len && chars[i].is_ascii_lowercase() {
                    i += 1;
                }
                words.push(s[start..i].to_string().to_lowercase());
            } else if c.is_ascii_digit() {
                // Match [0-9]+
                let start = i;
                i += 1;
                while i < len && chars[i].is_ascii_digit() {
                     i += 1;
                }
                words.push(s[start..i].to_string().to_lowercase());
            } else {
                // Delimiter or other character, skip
                i += 1;
            }
        }

        Name { words }
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.words.join("-"))
    }
}

impl Serialize for Name {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Classic format: ["word1", "word2"]
        serializer.collect_seq(&self.words)
    }
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct NameVisitor;

        impl<'de> Visitor<'de> for NameVisitor {
            type Value = Name;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an array of strings [\"word1\", \"word2\"]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let mut words = Vec::new();
                while let Some(word) = seq.next_element::<String>()? {
                    words.push(word);
                }
                Ok(Name { words })
            }
        }

        deserializer.deserialize_seq(NameVisitor)
    }
}

/// Classic Path - a list of Names, serialized as [["word1"], ["word2", "word3"]]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Path {
    pub segments: Vec<Name>,
}

impl Path {
    pub fn new(segments: Vec<Name>) -> Self {
        Self { segments }
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.segments.iter().map(|n| n.to_string()).collect();
        write!(f, "{}", parts.join("."))
    }
}

impl Serialize for Path {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Classic format: [["word1"], ["word2", "word3"]]
        serializer.collect_seq(&self.segments)
    }
}

impl<'de> Deserialize<'de> for Path {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PathVisitor;

        impl<'de> Visitor<'de> for PathVisitor {
            type Value = Path;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an array of Name arrays [[\"word1\"], [\"word2\"]]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let mut segments = Vec::new();
                while let Some(name) = seq.next_element::<Name>()? {
                    segments.push(name);
                }
                Ok(Path { segments })
            }
        }

        deserializer.deserialize_seq(PathVisitor)
    }
}

/// Classic FQName - fully qualified name (package path, module path, local name)
/// Serialized as [[[package_name]], [[module_name]], ["local_name"]]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
}

impl fmt::Display for FQName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.package_path, self.module_path, self.local_name
        )
    }
}

impl Serialize for FQName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Classic format: [[[package]], [[module]], ["name"]]
        let mut tuple = serializer.serialize_tuple(3)?;
        tuple.serialize_element(&self.package_path)?;
        tuple.serialize_element(&self.module_path)?;
        tuple.serialize_element(&self.local_name)?;
        tuple.end()
    }
}

impl<'de> Deserialize<'de> for FQName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FQNameVisitor;

        impl<'de> Visitor<'de> for FQNameVisitor {
            type Value = FQName;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a 3-element array [package_path, module_path, local_name]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let package_path = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let module_path = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let local_name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                
                if let Some(_) = seq.next_element::<serde_json::Value>()? {
                     return Err(de::Error::custom("Expected end of FQName array"));
                }

                Ok(FQName {
                    package_path,
                    module_path,
                    local_name,
                })
            }
        }

        deserializer.deserialize_seq(FQNameVisitor)
    }
}

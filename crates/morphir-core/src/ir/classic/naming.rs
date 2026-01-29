//! Classic IR Naming types
//!
//! Name, Path, and FQName for the Classic Morphir IR format.
//! These use array-based serialization instead of string-based.

use crate::naming::{Word, intern, resolve};
use schemars::JsonSchema;
use serde::de::{self, IgnoredAny, SeqAccess, Visitor};
use serde::ser::{SerializeTuple, Serializer};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

/// Classic Name - a list of words, serialized as ["word1", "word2"]
#[derive(Debug, Clone, PartialEq, Eq, Hash, JsonSchema)]
pub struct Name {
    #[schemars(with = "Vec<String>")]
    pub words: Vec<Word>,
}

impl std::str::FromStr for Name {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
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
                words.push(intern(&s[start..i].to_lowercase()));
            } else if c.is_ascii_digit() {
                // Match [0-9]+
                let start = i;
                i += 1;
                while i < len && chars[i].is_ascii_digit() {
                    i += 1;
                }
                words.push(intern(&s[start..i].to_lowercase()));
            } else {
                // Delimiter or other character, skip
                i += 1;
            }
        }

        Ok(Name { words })
    }
}

impl Name {
    pub fn new<I, S>(words: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            words: words.into_iter().map(|w| intern(w.as_ref())).collect(),
        }
    }

    /// Helper for from_str that unwraps since it's infallible
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        <Self as std::str::FromStr>::from_str(s).unwrap()
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, word) in self.words.iter().enumerate() {
            if i > 0 {
                write!(f, "-")?;
            }
            write!(f, "{}", resolve(*word))?;
        }
        Ok(())
    }
}

impl Serialize for Name {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Classic format: ["word1", "word2"]
        serializer.collect_seq(self.words.iter().map(|w| resolve(*w)))
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
                    words.push(intern(&word));
                }
                Ok(Name { words })
            }
        }

        deserializer.deserialize_seq(NameVisitor)
    }
}

/// Classic Path - a list of Names, serialized as [["word1"], ["word2", "word3"]]
#[derive(Debug, Clone, PartialEq, Eq, Hash, JsonSchema)]
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
        for (i, segment) in self.segments.iter().enumerate() {
            if i > 0 {
                write!(f, ".")?;
            }
            write!(f, "{}", segment)?;
        }
        Ok(())
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

                if seq.next_element::<IgnoredAny>()?.is_some() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_display() {
        let n = Name::from_str("foo-bar_baz");
        assert_eq!(n.to_string(), "foo-bar-baz");
    }

    #[test]
    fn test_path_display() {
        let p = Path::new(vec![Name::from_str("foo"), Name::from_str("bar")]);
        assert_eq!(p.to_string(), "foo.bar");
    }

    #[test]
    fn test_name_from_str_edge_cases() {
        assert_eq!(Name::from_str("").words, Vec::<Word>::new());
        assert_eq!(Name::from_str("123").words, vec![intern("123")]);
        assert_eq!(
            Name::from_str("ABC").words,
            vec![intern("a"), intern("b"), intern("c")]
        );
        assert_eq!(
            Name::from_str("a1b2").words,
            vec![intern("a"), intern("1"), intern("b"), intern("2")]
        );
    }
}

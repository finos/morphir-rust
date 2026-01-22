use serde::{Deserialize, Serialize};
use schemars::JsonSchema;
use std::fmt;

/// A Name is a list of words.
///
/// It facilitates conversion between different naming conventions (camelCase, snake_case, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Hash, JsonSchema)]
pub struct Name {
    pub words: Vec<String>,
}

impl Name {
    /// Creates a new Name from a string, parsing it into words based on common separators and casing.
    pub fn new(s: &str) -> Self {
        let words = parse_name_to_words(s);
        Self { words }
    }

    pub fn from_words(words: Vec<String>) -> Self {
        Self { words }
    }

    pub fn to_camel_case(&self) -> String {
        let mut chars = String::new();
        for (i, word) in self.words.iter().enumerate() {
            if i == 0 {
                chars.push_str(&word.to_lowercase());
            } else {
                chars.push_str(&to_title_case_word(word));
            }
        }
        chars
    }

    pub fn to_title_case(&self) -> String {
        self.words.iter().map(|w| to_title_case_word(w)).collect::<Vec<_>>().join("")
    }

    pub fn to_snake_case(&self) -> String {
        self.words.iter().map(|w| w.to_lowercase()).collect::<Vec<_>>().join("_")
    }

    pub fn to_kebab_case(&self) -> String {
        self.words.iter().map(|w| w.to_lowercase()).collect::<Vec<_>>().join("-")
    }
}

fn to_title_case_word(word: &str) -> String {
    let mut c = word.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

fn parse_name_to_words(s: &str) -> Vec<String> {
    let parts: Vec<&str> = s.split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()).collect();
    let mut words = Vec::new();

    for part in parts {
        let chars: Vec<char> = part.chars().collect();
        if chars.is_empty() { continue; }

        let mut start = 0;
        for i in 0..chars.len() {
            let c = chars[i];
            let has_next = i + 1 < chars.len();
            let next = if has_next { Some(chars[i+1]) } else { None };
            
            let split = if i == 0 {
                false 
            } else {
                 let prev = chars[i-1];
                 if prev.is_lowercase() && c.is_uppercase() {
                     true
                 } 
                 else if prev.is_uppercase() && c.is_uppercase() {
                     if let Some(n) = next {
                         if n.is_lowercase() {
                             true
                         } else {
                             false
                         }
                     } else {
                         false
                     }
                 } else {
                     false
                 }
            };

            if split {
                 words.push(part[start..i].to_lowercase());
                 start = i;
            }
        }
        words.push(part[start..].to_lowercase());
    }
    words
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_kebab_case())
    }
}

impl Serialize for Name {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_kebab_case())
    }
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum NameFormat {
            String(String),
            Array(Vec<String>),
        }

        let v = NameFormat::deserialize(deserializer)?;
        match v {
            NameFormat::String(s) => Ok(Name::new(&s)),
            NameFormat::Array(arr) => Ok(Name::from_words(arr)),
        }
    }
}

impl From<&str> for Name {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<Vec<String>> for Name {
    fn from(words: Vec<String>) -> Self {
        Self::from_words(words)
    }
}

impl AsRef<[String]> for Name {
    fn as_ref(&self) -> &[String] {
        &self.words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_parsing() {
        assert_eq!(Name::new("fooBar").words, vec!["foo", "bar"]);
        assert_eq!(Name::new("foo_bar").words, vec!["foo", "bar"]);
        assert_eq!(Name::new("foo-bar").words, vec!["foo", "bar"]);
        assert_eq!(Name::new("FooBar").words, vec!["foo", "bar"]);
    }

    #[test]
    fn test_name_abbreviations() {
        assert_eq!(Name::new("JSONResponse").words, vec!["json", "response"]);
        assert_eq!(Name::new("parseXML").words, vec!["parse", "xml"]);
        assert_eq!(Name::new("myXMLParser").words, vec!["my", "xml", "parser"]);
        assert_eq!(Name::new("HTTPServer").words, vec!["http", "server"]);
    }

    #[test]
    fn test_name_casing() {
        let name = Name::new("foo_bar_baz");
        assert_eq!(name.to_camel_case(), "fooBarBaz");
        assert_eq!(name.to_title_case(), "FooBarBaz");
        assert_eq!(name.to_snake_case(), "foo_bar_baz");
        assert_eq!(name.to_kebab_case(), "foo-bar-baz");
    }

    #[test]
    fn test_serialization_canonical() {
        let name = Name::new("fooBar");
        let json = serde_json::to_string(&name).unwrap();
        assert_eq!(json, "\"foo-bar\"");
    }

    #[test]
    fn test_deserialization_canonical() {
        let json = "\"foo-bar\"";
        let name: Name = serde_json::from_str(json).unwrap();
        assert_eq!(name.words, vec!["foo", "bar"]);
    }

    #[test]
    fn test_deserialization_legacy() {
        let json = "[\"foo\", \"bar\"]";
        let name: Name = serde_json::from_str(json).unwrap();
        assert_eq!(name.words, vec!["foo", "bar"]);
    }
}

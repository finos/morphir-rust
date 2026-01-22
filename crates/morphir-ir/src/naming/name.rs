use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, JsonSchema)]
pub struct Name {
    pub words: Vec<String>,
}

impl Name {
    /// Create a new Name from a slice of words
    pub fn new(words: &[&str]) -> Self {
        Name {
            words: words.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Parse a Name from a string (kebab-case, snake_case, camelCase, etc)
    pub fn from(name: &str) -> Self {
        let mut words = Vec::new();
        let mut current_word = String::new();

        for c in name.chars() {
            if c == '_' || c == '-' || c == '/' || c == '.' || c == ':' {
                if !current_word.is_empty() {
                    words.push(current_word);
                    current_word = String::new();
                }
            } else if c.is_uppercase() {
                if !current_word.is_empty() {
                    // Split on uppercase if we have a current word
                    words.push(current_word);
                    current_word = String::new();
                }
                current_word.push(c);
            } else {
                current_word.push(c);
            }
        }
        if !current_word.is_empty() {
            words.push(current_word);
        }
        
        Name { words }
    }

    pub fn to_camel_case(&self) -> String {
        let mut result = String::new();
        for (i, word) in self.words.iter().enumerate() {
            if i == 0 {
                result.push_str(&word.to_lowercase());
            } else {
                let mut chars = word.chars();
                if let Some(first) = chars.next() {
                    result.push(first.to_ascii_uppercase());
                    result.push_str(chars.as_str());
                }
            }
        }
        result
    }

    pub fn to_snake_case(&self) -> String {
        self.words.join("_").to_lowercase()
    }

    pub fn to_kebab_case(&self) -> String {
        self.words.join("-").to_lowercase()
    }

    pub fn to_title_case(&self) -> String {
        self.words
            .iter()
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<Vec<String>>()
            .join("")
    }

    pub fn is_lowercase(&self) -> bool {
         self.to_snake_case().chars().all(|c| c.is_lowercase() || c == '_')
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Use kebab-case for canonical string representation to match test expectations
        // and standard path formatting
        write!(f, "{}", self.to_kebab_case())
    }
}

impl Serialize for Name {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.words.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let words = Vec::<String>::deserialize(deserializer)?;
        Ok(Name { words })
    }
}

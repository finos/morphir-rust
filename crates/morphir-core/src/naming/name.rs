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

    /// Render this name using the Morphir IR v4 canonical encoding.
    ///
    /// Consecutive one-letter words are grouped in parentheses so decoding can
    /// distinguish an acronym such as `["u", "s", "d"]` from `["usd"]`.
    pub fn to_canonical_string(&self) -> String {
        fn push_acronym(parts: &mut Vec<String>, acronym: &mut String) {
            if acronym.len() == 1 {
                parts.push(std::mem::take(acronym));
            } else if !acronym.is_empty() {
                parts.push(format!("({acronym})"));
                acronym.clear();
            }
        }

        let mut parts = Vec::new();
        let mut acronym = String::new();

        for word in &self.words {
            if word.chars().count() == 1 {
                acronym.push_str(&word.to_lowercase());
            } else {
                push_acronym(&mut parts, &mut acronym);
                parts.push(word.to_lowercase());
            }
        }

        push_acronym(&mut parts, &mut acronym);

        parts.join("-")
    }

    /// Parse a Morphir IR v4 canonical name.
    pub fn from_canonical_string(source: &str) -> Result<Self, String> {
        if source.is_empty() {
            return Ok(Self { words: Vec::new() });
        }

        let mut words = Vec::new();
        for part in source.split('-') {
            if part.is_empty() {
                return Err(format!("empty word in canonical name: {source}"));
            }

            let opens = part.starts_with('(');
            let closes = part.ends_with(')');
            let nested_parentheses =
                part.len() > 2 && part[1..part.len().saturating_sub(1)].contains(['(', ')']);
            if opens != closes || nested_parentheses {
                return Err(format!("unmatched parentheses in canonical name: {source}"));
            }

            if opens {
                let acronym = &part[1..part.len() - 1];
                if acronym.is_empty() {
                    return Err(format!("empty acronym in canonical name: {source}"));
                }
                words.extend(acronym.chars().map(|character| character.to_string()));
            } else if part.contains(['(', ')']) {
                return Err(format!("unmatched parentheses in canonical name: {source}"));
            } else {
                words.push(part.to_owned());
            }
        }

        Ok(Self { words })
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
        self.to_snake_case()
            .chars()
            .all(|c| c.is_lowercase() || c == '_')
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_canonical_string())
    }
}

impl Serialize for Name {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_canonical_string())
    }
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de;

        // Accept both array format (Classic) and string format (V4)
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            // V4 canonical string format: "testModule" or "my-function"
            serde_json::Value::String(s) => {
                Name::from_canonical_string(&s).map_err(de::Error::custom)
            }
            // Classic array format: ["test", "module"]
            serde_json::Value::Array(arr) => {
                let words: Result<Vec<String>, _> = arr
                    .into_iter()
                    .map(|v| match v {
                        serde_json::Value::String(s) => Ok(s),
                        _ => Err(de::Error::custom("expected string in Name array")),
                    })
                    .collect();
                Ok(Name { words: words? })
            }
            _ => Err(de::Error::custom("expected string or array for Name")),
        }
    }
}

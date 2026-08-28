//! YAML frontmatter: fence splitting, permissive parsing, typed accessors.
//!
//! Ported from `KbStore.splitFrontmatter` / `parseFrontmatter` and the
//! `Frontmatter` model in `KbModel.scala`. Parsing rejects duplicate keys
//! (serde_yaml 0.9 does this at any nesting level, matching SnakeYAML with
//! `allowDuplicateKeys = false`); accessors are deliberately permissive so
//! that a missing or wrongly-typed field yields `None`/empty rather than an
//! error, and `kb check` can report every problem in one pass.

use serde_yaml::Value;

use crate::model::SourceRef;

/// Splits a leading `---` fenced YAML block off the front of a document.
///
/// Returns `(raw_frontmatter_or_none, body)`. Line endings are normalized
/// CRLF→LF first. The opening fence must be the literal first four bytes
/// `---\n`; the closing fence is the first subsequent line whose trimmed
/// content is `---`. A document with no opening fence, or an unterminated
/// fence, yields `(None, whole_normalized_text)`.
pub fn split_frontmatter(text: &str) -> (Option<String>, String) {
    let normalized = text.replace("\r\n", "\n");
    if !normalized.starts_with("---\n") {
        return (None, normalized);
    }
    let rest = &normalized[4..];
    match find_closing_fence(rest) {
        Some(i) => {
            let raw = rest[..i].to_string();
            let after = &rest[i..];
            let body = match after.find('\n') {
                Some(n) => after[n + 1..].to_string(),
                None => String::new(),
            };
            (Some(raw), body)
        }
        None => (None, normalized),
    }
}

/// Byte offset of the start of the first line in `s` whose trim is `---`.
fn find_closing_fence(s: &str) -> Option<usize> {
    let mut idx = 0;
    loop {
        let line_end = s[idx..].find('\n').map(|n| idx + n).unwrap_or(s.len());
        if s[idx..line_end].trim() == "---" {
            return Some(idx);
        }
        if line_end >= s.len() {
            return None;
        }
        idx = line_end + 1;
    }
}

/// How many file lines a stripped frontmatter block occupied: the raw block's
/// newline count plus the two fence lines. Body-relative line numbers are
/// shifted by this to refer to the original file.
pub fn frontmatter_line_count(raw_fm: &str) -> usize {
    raw_fm.matches('\n').count() + 2
}

/// Parses a YAML frontmatter block.
///
/// Returns `Err` with the parser's message (first line only) when the YAML is
/// malformed — including when it contains duplicate mapping keys — or when the
/// document is not a mapping. An empty block parses as empty frontmatter.
pub fn parse_frontmatter(raw: &str) -> std::result::Result<Frontmatter, String> {
    let value: Value = serde_yaml::from_str(raw)
        .map_err(|e| e.to_string().lines().next().unwrap_or_default().to_string())?;
    match value {
        Value::Null => Ok(Frontmatter::empty()),
        Value::Mapping(m) => Ok(Frontmatter::from_mapping(m)),
        other => Err(format!(
            "frontmatter is {}, expected a mapping",
            kind_name(&other)
        )),
    }
}

fn kind_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Sequence(_) => "a sequence",
        Value::Mapping(_) => "a mapping",
        Value::Tagged(_) => "a tagged value",
    }
}

/// How a YAML scalar reads as a string. Shared by every accessor so a value
/// means the same thing whether it stands alone or sits in a list:
/// `supersedes: [2]` parses as a number, and an accessor keeping only strings
/// would drop it silently, leaving supersession unvalidated.
///
/// Unlike SnakeYAML, serde_yaml never resolves an unquoted `2026-07-28` to a
/// date type — it stays a string — so date-valued fields (OKF's `stale_after`,
/// intent's `created`/`state_since`) read back verbatim through the string
/// arm. A regression test pins that behavior.
pub fn scalar_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Parsed YAML frontmatter, kept as raw values with typed accessors on top.
///
/// Entries preserve document order. Accessors are permissive: a missing or
/// wrongly-typed field yields `None`/empty rather than an error.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Frontmatter {
    entries: Vec<(String, Value)>,
}

impl Frontmatter {
    /// Frontmatter with no entries.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Builds frontmatter from a parsed YAML mapping, coercing keys to
    /// strings. Duplicate keys were already rejected by the parser.
    pub fn from_mapping(mapping: serde_yaml::Mapping) -> Self {
        let entries = mapping
            .into_iter()
            .map(|(k, v)| {
                let key = scalar_to_string(&k).unwrap_or_else(|| format!("{k:?}"));
                (key, v)
            })
            .collect();
        Self { entries }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Keys in document order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.as_str())
    }

    /// The raw YAML value at `key`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// The value at `key` as a string, coercing scalars.
    pub fn str_at(&self, key: &str) -> Option<String> {
        self.get(key).and_then(scalar_to_string)
    }

    /// The value at `key` as a list of strings. A sequence collects its
    /// scalar elements; a bare scalar reads as a one-element list.
    pub fn list_at(&self, key: &str) -> Vec<String> {
        match self.get(key) {
            Some(Value::Sequence(items)) => items.iter().filter_map(scalar_to_string).collect(),
            Some(v) => scalar_to_string(v).into_iter().collect(),
            None => Vec::new(),
        }
    }

    /// The value at `key` as a boolean; a `"true"`/`"false"` string counts.
    pub fn bool_at(&self, key: &str) -> Option<bool> {
        match self.get(key) {
            Some(Value::Bool(b)) => Some(*b),
            Some(Value::String(s)) => s.parse().ok(),
            _ => None,
        }
    }

    /// The value at `key` as an integer; a numeric string counts.
    pub fn int_at(&self, key: &str) -> Option<i64> {
        match self.get(key) {
            Some(Value::Number(n)) => n.as_i64(),
            Some(Value::String(s)) => s.parse().ok(),
            _ => None,
        }
    }

    /// The mappings at `key`: a sequence collects its mapping elements, a
    /// bare mapping reads as a one-element list.
    pub fn maps_at(&self, key: &str) -> Vec<&serde_yaml::Mapping> {
        match self.get(key) {
            Some(Value::Sequence(items)) => items.iter().filter_map(|v| v.as_mapping()).collect(),
            Some(Value::Mapping(m)) => vec![m],
            _ => Vec::new(),
        }
    }

    pub fn doc_type(&self) -> Option<String> {
        self.str_at("type")
    }

    pub fn title(&self) -> Option<String> {
        self.str_at("title")
    }

    pub fn description(&self) -> Option<String> {
        self.str_at("description")
    }

    pub fn resource(&self) -> Option<String> {
        self.str_at("resource")
    }

    pub fn status(&self) -> Option<String> {
        self.str_at("status")
    }

    pub fn stale_after(&self) -> Option<String> {
        self.str_at("stale_after")
    }

    pub fn okf_version(&self) -> Option<String> {
        self.str_at("okf_version")
    }

    pub fn tags(&self) -> Vec<String> {
        self.list_at("tags")
    }

    /// Provenance entries from the `sources` family. An entry without a
    /// string `resource` is dropped; `id` and `title` must be strings, as in
    /// the reference implementation.
    pub fn sources(&self) -> Vec<SourceRef> {
        self.maps_at("sources")
            .into_iter()
            .filter_map(|m| {
                let resource = str_field(m, "resource")?;
                Some(SourceRef {
                    id: str_field(m, "id"),
                    resource,
                    title: str_field(m, "title"),
                })
            })
            .collect()
    }
}

fn str_field(m: &serde_yaml::Mapping, key: &str) -> Option<String> {
    match m.get(Value::String(key.to_string()))? {
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

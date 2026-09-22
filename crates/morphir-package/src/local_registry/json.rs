use super::diagnostics::{invalid, resource};
use super::{Diagnostic, Phase, Resource, Rule, Subject};
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;
use std::{collections::BTreeMap, fmt};
/// Syntax domain determines byte limits and number-token rules.
#[derive(Debug, Clone, Copy)]
pub enum JsonDomain {
    /// Library lock.
    Lock,
    /// Canonical registry record.
    Record,
    /// Canonical release statement payload.
    Statement,
    /// Trusted local policy.
    Policy,
    /// Open DSSE envelope.
    Dsse,
    /// Open TUF metadata.
    Tuf(TufRole),
}
/// Supported TUF metadata role, for syntax interpretation only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TufRole {
    /// Root metadata.
    Root,
    /// Timestamp metadata.
    Timestamp,
    /// Snapshot metadata.
    Snapshot,
    /// Targets metadata.
    Targets,
}
/// Lossless JSON syntax. This is not trusted package state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonNode {
    /// JSON null.
    Null,
    /// Boolean leaf.
    Boolean(bool),
    /// Decoded Unicode text.
    String(String),
    /// Original JSON number token.
    Number(String),
    /// Ordered array.
    Array(Vec<JsonNode>),
    /// Object with unique decoded keys.
    Object(BTreeMap<String, JsonNode>),
}
impl JsonNode {
    /// Look up an object member.
    pub fn get(&self, key: &str) -> Option<&Self> {
        if let Self::Object(m) = self {
            m.get(key)
        } else {
            None
        }
    }
    /// Borrow a decoded string.
    pub fn as_str(&self) -> Option<&str> {
        if let Self::String(s) = self {
            Some(s)
        } else {
            None
        }
    }
    /// Borrow an array.
    pub fn as_array(&self) -> Option<&[Self]> {
        if let Self::Array(a) = self {
            Some(a)
        } else {
            None
        }
    }
    /// Borrow an exact number token.
    pub fn number_text(&self) -> Option<&str> {
        if let Self::Number(s) = self {
            Some(s)
        } else {
            None
        }
    }
}
/// Owned input text and lossless syntax after bounded byte decoding.
#[derive(Debug, Clone)]
pub struct DecodedJson {
    pub(crate) text: String,
    pub(crate) document: JsonNode,
}
impl DecodedJson {
    /// Exact decoded UTF-8 text.
    pub fn text(&self) -> &str {
        &self.text
    }
    /// Lossless JSON syntax.
    pub fn document(&self) -> &JsonNode {
        &self.document
    }
}
/// Decode bounded bytes before interpreting a document's typed shape.
///
/// ```
/// use morphir_package::local_registry::*;
/// let value = decode_json_domain(b"{}", JsonDomain::Dsse, &Subject::Lock, Phase::Decode)?;
/// assert_eq!(value.text(), "{}");
/// # Ok::<(), Diagnostic>(())
/// ```
pub fn decode_json_domain(
    bytes: &[u8],
    domain: JsonDomain,
    subject: &Subject,
    phase: Phase,
) -> Result<DecodedJson, Diagnostic> {
    let res = match domain {
        JsonDomain::Lock => Resource::LockBytes,
        JsonDomain::Record => Resource::RecordBytes,
        JsonDomain::Statement => Resource::StatementBytes,
        JsonDomain::Policy => Resource::PolicyBytes,
        JsonDomain::Dsse => Resource::EnvelopeBytes,
        JsonDomain::Tuf(role) => match role {
            TufRole::Root => Resource::RootBytes,
            TufRole::Timestamp => Resource::TimestampBytes,
            TufRole::Snapshot => Resource::SnapshotBytes,
            TufRole::Targets => Resource::TargetsBytes,
        },
    };
    let maximum = if matches!(res, Resource::LockBytes | Resource::TargetsBytes) {
        16_777_216
    } else {
        1_048_576
    };
    if bytes.len() > maximum {
        return Err(resource(subject, phase, res, maximum));
    }
    let malformed = || invalid(subject, phase, vec![(String::new(), Rule::MalformedJson)]);
    let text = std::str::from_utf8(bytes).map_err(|_| malformed())?;
    if text.starts_with('\u{feff}') {
        return Err(malformed());
    }
    if exceeds_depth(text) {
        return Err(resource(subject, phase, Resource::JsonDepth, 64));
    }
    let raw: &RawValue = serde_json::from_str(text).map_err(|_| malformed())?;
    let mut duplicates = Vec::new();
    let document = parse_raw(raw, "", &mut duplicates).map_err(|_| malformed())?;
    if !duplicates.is_empty() {
        return Err(invalid(
            subject,
            phase,
            duplicates
                .into_iter()
                .map(|p| (p, Rule::DuplicateKey))
                .collect(),
        ));
    }
    if matches!(domain, JsonDomain::Tuf(_)) {
        let mut violations = Vec::new();
        number_violations(&document, "", &mut violations);
        if !violations.is_empty() {
            return Err(invalid(subject, phase, violations));
        }
    }
    Ok(DecodedJson {
        text: text.to_owned(),
        document,
    })
}
fn exceeds_depth(text: &str) -> bool {
    let (mut depth, mut quoted, mut escaped) = (0i32, false, false);
    for c in text.chars() {
        if quoted {
            if escaped {
                escaped = false
            } else if c == '\\' {
                escaped = true
            } else if c == '"' {
                quoted = false
            }
            continue;
        }
        if matches!(c, '}' | ']') {
            depth -= 1;
            continue;
        }
        if " \t\r\n,:".contains(c) {
            continue;
        }
        if depth > 64 {
            return true;
        }
        if c == '"' {
            quoted = true
        } else if matches!(c, '{' | '[') {
            depth += 1
        }
    }
    false
}
// Borrow raw subtrees so nested inputs do not allocate another copy of their
// remaining text at every level. Only the final lossless syntax owns leaf data.
struct Members<'a>(Vec<(String, &'a RawValue)>);
impl<'de> Deserialize<'de> for Members<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Members<'de>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON object")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Members<'de>, M::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry)
                }
                Ok(Members(entries))
            }
        }
        d.deserialize_map(V)
    }
}
pub(crate) fn child(p: &str, k: impl fmt::Display) -> String {
    format!(
        "{p}/{}",
        k.to_string().replace('~', "~0").replace('/', "~1")
    )
}
fn parse_raw(
    raw: &RawValue,
    p: &str,
    duplicates: &mut Vec<String>,
) -> Result<JsonNode, serde_json::Error> {
    Ok(match raw.get().as_bytes()[0] {
        b'{' => {
            let entries: Members = serde_json::from_str(raw.get())?;
            let mut map = BTreeMap::new();
            for (k, v) in entries.0 {
                let pointer = child(p, &k);
                let value = parse_raw(v, &pointer, duplicates)?;
                if map.insert(k, value).is_some() {
                    duplicates.push(pointer)
                }
            }
            JsonNode::Object(map)
        }
        b'[' => {
            let entries: Vec<&RawValue> = serde_json::from_str(raw.get())?;
            JsonNode::Array(
                entries
                    .iter()
                    .enumerate()
                    .map(|(i, v)| parse_raw(v, &child(p, i), duplicates))
                    .collect::<Result<_, _>>()?,
            )
        }
        b'"' => JsonNode::String(serde_json::from_str(raw.get())?),
        b'n' => JsonNode::Null,
        b't' => JsonNode::Boolean(true),
        b'f' => JsonNode::Boolean(false),
        _ => JsonNode::Number(raw.get().to_owned()),
    })
}
fn number_violations(v: &JsonNode, p: &str, out: &mut Vec<(String, Rule)>) {
    match v {
        JsonNode::Number(s) if s.contains(['.', 'e', 'E']) => {
            out.push((p.into(), Rule::InvalidValue))
        }
        JsonNode::Object(m) => {
            for (k, v) in m {
                number_violations(v, &child(p, k), out)
            }
        }
        JsonNode::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                number_violations(v, &child(p, i), out)
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_raw_members_borrow_original_text_without_subtree_copies() {
        // A moderate payload at the maximum scalar depth exposes the allocation
        // pattern without an OOM test or process-wide allocator instrumentation.
        let text = format!(
            "{}\"{}\"{}",
            "{\"child\":".repeat(64),
            "x".repeat(32_768),
            "}".repeat(64)
        );
        let range = text.as_ptr() as usize..text.as_ptr() as usize + text.len();
        fn inspect(raw: &RawValue, range: &std::ops::Range<usize>) {
            if raw.get().starts_with('{') {
                let members: Members = serde_json::from_str(raw.get()).unwrap();
                for (_, value) in members.0 {
                    assert!(
                        range.contains(&(value.get().as_ptr() as usize)),
                        "nested raw subtree was copied into a separate allocation"
                    );
                    inspect(value, range);
                }
            }
        }
        inspect(serde_json::from_str(&text).unwrap(), &range);
    }
}

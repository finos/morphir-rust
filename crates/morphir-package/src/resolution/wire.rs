use super::model::{Violation, ViolationRule};
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value, value::RawValue};
use std::collections::BTreeSet;
use std::fmt;

pub(super) enum DocumentError {
    Malformed,
    ResourceExhausted,
}

pub(super) struct Document {
    pub(super) value: Value,
    pub(super) duplicates: Vec<Violation>,
}

pub(super) fn parse_document(input: &str) -> Result<Document, DocumentError> {
    if input.starts_with('\u{feff}') {
        return Err(DocumentError::Malformed);
    }
    crate::strict_json::on_decode_stack(|| {
        let raw: &RawValue = serde_json::from_str(input).map_err(|_| DocumentError::Malformed)?;
        let mut duplicate_pointers = BTreeSet::new();
        let value = parse_raw(raw, "", 0, &mut duplicate_pointers)?;
        let duplicates = duplicate_pointers
            .into_iter()
            .map(|pointer| Violation {
                pointer,
                rule: ViolationRule::DuplicateKey,
            })
            .collect();
        Ok(Document { value, duplicates })
    })
}

struct Members(Vec<(String, Box<RawValue>)>);

impl<'de> Deserialize<'de> for Members {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct AllMembers;
        impl<'de> Visitor<'de> for AllMembers {
            type Value = Members;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Members, M::Error> {
                let mut members = Vec::new();
                while let Some(member) = map.next_entry::<String, Box<RawValue>>()? {
                    members.push(member);
                }
                Ok(Members(members))
            }
        }
        deserializer.deserialize_map(AllMembers)
    }
}

fn parse_raw(
    raw: &RawValue,
    pointer: &str,
    depth: usize,
    duplicate_pointers: &mut BTreeSet<String>,
) -> Result<Value, DocumentError> {
    let first = raw.get().as_bytes().first().copied();
    if depth >= 1000 && matches!(first, Some(b'{' | b'[')) {
        return Err(DocumentError::ResourceExhausted);
    }
    match first {
        Some(b'{') => {
            let Members(members) =
                serde_json::from_str(raw.get()).map_err(|_| DocumentError::Malformed)?;
            let mut seen = BTreeSet::new();
            let mut object = Map::new();
            for (key, value) in members {
                let child_pointer = join_pointer(pointer, &key);
                if !seen.insert(key.clone()) {
                    duplicate_pointers.insert(child_pointer.clone());
                }
                let value = parse_raw(&value, &child_pointer, depth + 1, duplicate_pointers)?;
                object.entry(key).or_insert(value);
            }
            Ok(Value::Object(object))
        }
        Some(b'[') => {
            let values: Vec<Box<RawValue>> =
                serde_json::from_str(raw.get()).map_err(|_| DocumentError::Malformed)?;
            values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    parse_raw(
                        value,
                        &format!("{pointer}/{index}"),
                        depth + 1,
                        duplicate_pointers,
                    )
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        Some(_) => serde_json::from_str(raw.get()).map_err(|_| DocumentError::Malformed),
        None => Err(DocumentError::Malformed),
    }
}

pub(super) fn join_pointer(parent: &str, member: &str) -> String {
    let escaped = member.replace('~', "~0").replace('/', "~1");
    format!("{parent}/{escaped}")
}

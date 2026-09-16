//! JSON parsing that preserves numbers and refuses duplicate decoded object keys.

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, value::RawValue};
use std::collections::BTreeMap;
use std::fmt;

/// Parse JSON without silently replacing duplicate members.
///
/// Raw values distinguish actual JSON objects from serde_json's private number
/// token under `arbitrary_precision`. Even a key equal to that token is ordinary data.
pub fn parse(text: &str) -> Result<Value, serde_json::Error> {
    on_decode_stack(|| {
        let raw: &RawValue = serde_json::from_str(text)?;
        parse_raw(raw, 0)
    })
}

// The current IR JSON codec permits 1000 containers. Recursive deserialization at
// that depth needs a stated stack size instead of the platform's main-thread default.
pub(crate) fn on_decode_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn_scoped(scope, f)
            .expect("a package decode thread")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

struct Members(BTreeMap<String, Box<RawValue>>);

impl<'de> Deserialize<'de> for Members {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Unique;
        impl<'de> Visitor<'de> for Unique {
            type Value = Members;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("object with unique decoded keys")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Members, M::Error> {
                let mut members = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, Box<RawValue>>()? {
                    if members.insert(key, value).is_some() {
                        return Err(serde::de::Error::custom("duplicate object key"));
                    }
                }
                Ok(Members(members))
            }
        }
        deserializer.deserialize_map(Unique)
    }
}

fn parse_raw(raw: &RawValue, depth: usize) -> Result<Value, serde_json::Error> {
    if depth >= 1000 && matches!(raw.get().as_bytes()[0], b'{' | b'[') {
        return Err(<serde_json::Error as serde::de::Error>::custom(
            "JSON nesting exceeds 1000 containers",
        ));
    }
    match raw.get().as_bytes()[0] {
        b'{' => {
            let members: Members = serde_json::from_str(raw.get())?;
            members
                .0
                .into_iter()
                .map(|(key, value)| Ok((key, parse_raw(&value, depth + 1)?)))
                .collect::<Result<_, _>>()
                .map(Value::Object)
        }
        b'[' => {
            let values: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
            values
                .iter()
                .map(|value| parse_raw(value, depth + 1))
                .collect::<Result<_, _>>()
                .map(Value::Array)
        }
        _ => serde_json::from_str(raw.get()),
    }
}

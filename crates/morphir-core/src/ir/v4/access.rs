//! Access control types for Morphir IR V4

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Access control
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Access {
    Public,
    Private,
}

impl Access {
    /// The canonical variant tag this access level is written with.
    pub fn tag(self) -> &'static str {
        match self {
            Access::Public => "Public",
            Access::Private => "Private",
        }
    }

    /// The access level a wrapper tag names, if it names one.
    ///
    /// `Public` and `Private` are the canonical tags; `pub`, `public` and `private` are the
    /// shorthands a reader accepts silently beside them. `priv` is not an access spelling
    /// (definitions-0031).
    pub fn from_tag(tag: &str) -> Option<Access> {
        match tag {
            "Public" | "pub" | "public" => Some(Access::Public),
            "Private" | "private" => Some(Access::Private),
            _ => None,
        }
    }
}

/// Generic wrapper for access-controlled values.
///
/// The canonical spelling is the access level as the variant tag with the controlled value as
/// its payload: `{ "Public": { "TypeAliasDefinition": { … } } }`. A reader also accepts the
/// access level as a flattened member beside the value (`{ "access": "Public", … }`), the same
/// with the value nested under `value`, and the `pub`/`private` shorthands — all silently.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessControlled<T> {
    pub access: Access,
    pub value: T,
}

impl<T: Serialize> Serialize for AccessControlled<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry(self.access.tag(), &self.value)?;
        map.end()
    }
}

impl<'de, T> Deserialize<'de> for AccessControlled<T>
where
    T: for<'value> Deserialize<'value>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use super::serde_document::{carried, recover};

        let value = serde_json::Value::deserialize(deserializer)?;
        super::serde_document::decode_access_controlled(&value, "", |payload, cursor| {
            serde_json::from_value::<T>(payload.clone()).map_err(|error| recover(&error, cursor))
        })
        .map_err(carried)
    }
}

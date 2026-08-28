//! Access control types for Morphir IR V4

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Access control
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum Access {
    Public,
    Private,
}

/// Generic wrapper for access-controlled values
///
/// This matches morphir-elm's AccessControlled type, which is a generic wrapper
/// that can be applied to any type that needs access control.
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
        #[derive(Serialize)]
        struct Repr<'a, T> {
            access: &'a Access,
            value: &'a T,
        }
        Repr {
            access: &self.access,
            value: &self.value,
        }
        .serialize(serializer)
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
        let mut object = serde_json::Value::deserialize(deserializer)?;
        let object = object
            .as_object_mut()
            .ok_or_else(|| serde::de::Error::custom("expected an access-controlled object"))?;
        let access = object
            .remove("access")
            .ok_or_else(|| serde::de::Error::missing_field("access"))?;
        let access = serde_json::from_value(access).map_err(serde::de::Error::custom)?;
        let value = if let Some(value) = object.remove("value") {
            value
        } else {
            serde_json::Value::Object(std::mem::take(object))
        };
        let value = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self { access, value })
    }
}

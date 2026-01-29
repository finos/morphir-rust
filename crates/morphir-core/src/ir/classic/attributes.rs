use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Attributes in Morphir IR.
///
/// In Morphir IR V3, empty attributes are represented as `{}` in JSON.
/// This type ensures that the unit attributes are correctly serialized and deserialized.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Attributes;

impl Serialize for Attributes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_map(Some(0))?.end()
    }
}

impl<'de> Deserialize<'de> for Attributes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct AttributesVisitor;

        impl<'de> Visitor<'de> for AttributesVisitor {
            type Value = Attributes;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an empty map {}")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                while let Some(serde::de::IgnoredAny) = map.next_key()? {
                    let _: serde::de::IgnoredAny = map.next_value()?;
                }
                Ok(Attributes)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Attributes)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                while let Some(serde::de::IgnoredAny) = seq.next_element()? {}
                Ok(Attributes)
            }
        }

        deserializer.deserialize_any(AttributesVisitor)
    }
}

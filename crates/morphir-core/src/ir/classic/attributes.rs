use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Attributes in Morphir IR.
///
/// A specialized Option-like type for attributes with custom serialization behavior.
/// In Morphir IR V3, empty attributes are represented as `{}` in JSON.
/// This type is similar to `Option<A>` but with special serialization handling.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Attrs<A = ()> {
    None,
    Some(A),
}

impl<A: Serialize> Serialize for Attrs<A> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Attrs::None => serializer.serialize_map(Some(0))?.end(),
            Attrs::Some(value) => value.serialize(serializer),
        }
    }
}

/// Custom deserialization implementation for Attributes.
///
/// This implementation accepts multiple JSON representations.
/// The flexible deserialization ensures compatibility with various Morphir IR sources and formats.
///
/// # Accepted Formats for `Attrs::None`
///
/// 1. **Empty map** (canonical): `{}`
///    - This is the standard representation in Morphir IR V3
///
/// 2. **Non-empty map**: `{"key": "value", "another": "field"}`
///    - Any key-value pairs are deserialized as `Attrs::Some(A)` if `A` can be deserialized from a map
///    - Otherwise ignored and treated as `Attrs::None`
///
/// 3. **Unit value**: `null` or unit representation
///    - Accepts unit/null values from some serialization formats as `Attrs::None`
///
/// 4. **Empty sequence**: `[]`
///    - Accepts empty arrays for compatibility with alternative representations as `Attrs::None`
///
/// # Implementation Details
///
/// Uses a custom `Visitor` that:
/// - Implements `visit_map` to deserialize map entries into `A` or return `Attrs::None` if empty
/// - Implements `visit_unit` to accept null/unit values as `Attrs::None`
/// - Implements `visit_seq` to handle sequences (empty = None, non-empty tries to deserialize as `A`)
/// - Uses `deserialize_any` to handle all possible input formats flexibly
impl<'de, A: Deserialize<'de>> Deserialize<'de> for Attrs<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct AttributesVisitor<A>(std::marker::PhantomData<A>);

        impl<'de, A: Deserialize<'de>> Visitor<'de> for AttributesVisitor<A> {
            type Value = Attrs<A>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an empty map {} or a valid attribute value")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                // Peek to see if the map is empty
                use serde::de::Error;

                // Collect all entries to check if empty
                let mut entries = Vec::new();
                while let Some(key) = map.next_key::<serde_json::Value>()? {
                    let value = map.next_value::<serde_json::Value>()?;
                    entries.push((key, value));
                }

                if entries.is_empty() {
                    Ok(Attrs::None)
                } else {
                    // Reconstruct the map and deserialize
                    let map_value = serde_json::Value::Object(
                        entries.into_iter()
                            .filter_map(|(k, v)| k.as_str().map(|s| (s.to_string(), v)))
                            .collect()
                    );
                    let value = A::deserialize(map_value).map_err(Error::custom)?;
                    Ok(Attrs::Some(value))
                }
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Attrs::None)
            }

            fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
            where
                S: serde::de::SeqAccess<'de>,
            {
                // Check if sequence is empty
                if let Some(first) = seq.next_element::<serde_json::Value>()? {
                    // Sequence has elements, collect them all and deserialize
                    let mut elements = vec![first];
                    while let Some(elem) = seq.next_element()? {
                        elements.push(elem);
                    }
                    let seq_value = serde_json::Value::Array(elements);
                    let value = A::deserialize(seq_value).map_err(serde::de::Error::custom)?;
                    Ok(Attrs::Some(value))
                } else {
                    Ok(Attrs::None)
                }
            }
        }

        deserializer.deserialize_any(AttributesVisitor(std::marker::PhantomData))
    }
}

impl From<()> for Attrs {
    fn from(_: ()) -> Self {
        Attrs::None
    }
}

impl Default for Attrs {
    fn default() -> Self {
        Attrs::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_serialization() {
        let attrs: Attrs = Attrs::None;
        let json = serde_json::to_string(&attrs).expect("Failed to serialize");
        assert_eq!(json, "{}");
    }

    #[test]
    fn test_deserialization_empty_map() {
        let json = "{}";
        let attrs: Attrs = serde_json::from_str(json).expect("Failed to deserialize");
        assert_eq!(attrs, Attrs::None);
    }

    #[test]
    fn test_deserialization_non_empty_map_with_unit() {
        // With unit type (), non-empty maps fail to deserialize because () cannot be deserialized from a map
        // This is expected behavior - use Attrs<CustomType> if you need to deserialize non-empty maps
        let json = r#"{"key":"value","another":"field"}"#;
        let result: Result<Attrs, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_roundtrip_serialization() {
        let original: Attrs = Attrs::None;
        let serialized = serde_json::to_string(&original).expect("Failed to serialize");
        let deserialized: Attrs = serde_json::from_str(&serialized).expect("Failed to deserialize");
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_from_unit() {
        let attrs: Attrs = ().into();
        assert_eq!(attrs, Attrs::None);
    }

    #[test]
    fn test_from_unit_explicit() {
        let attrs = Attrs::from(());
        assert_eq!(attrs, Attrs::None);
    }

    #[test]
    fn test_default() {
        let attrs = Attrs::default();
        assert_eq!(attrs, Attrs::None);
    }

    #[test]
    fn test_some_serialization() {
        use serde_json::json;

        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct CustomAttrs {
            name: String,
            version: i32,
        }

        let attrs = Attrs::Some(CustomAttrs {
            name: "test".to_string(),
            version: 1,
        });

        let json = serde_json::to_string(&attrs).expect("Failed to serialize");
        let expected = json!({"name": "test", "version": 1});
        assert_eq!(json, serde_json::to_string(&expected).unwrap());
    }

    #[test]
    fn test_some_deserialization() {
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct CustomAttrs {
            name: String,
            version: i32,
        }

        let json = r#"{"name":"test","version":1}"#;
        let attrs: Attrs<CustomAttrs> = serde_json::from_str(json).expect("Failed to deserialize");

        assert_eq!(attrs, Attrs::Some(CustomAttrs {
            name: "test".to_string(),
            version: 1,
        }));
    }

    #[test]
    fn test_some_roundtrip() {
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct CustomAttrs {
            name: String,
            version: i32,
        }

        let original = Attrs::Some(CustomAttrs {
            name: "test".to_string(),
            version: 1,
        });

        let serialized = serde_json::to_string(&original).expect("Failed to serialize");
        let deserialized: Attrs<CustomAttrs> = serde_json::from_str(&serialized).expect("Failed to deserialize");

        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_none_with_custom_type() {
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct CustomAttrs {
            name: String,
        }

        let json = "{}";
        let attrs: Attrs<CustomAttrs> = serde_json::from_str(json).expect("Failed to deserialize");
        assert_eq!(attrs, Attrs::None);
    }

    #[test]
    fn test_some_serializes_as_underlying_struct() {
        // This test demonstrates that Attrs::Some(struct) serializes directly as the struct,
        // not wrapped in any additional structure
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct MyAttrs {
            author: String,
            version: String,
        }

        let attrs = Attrs::Some(MyAttrs {
            author: "John Doe".to_string(),
            version: "1.0.0".to_string(),
        });

        let json = serde_json::to_string(&attrs).expect("Failed to serialize");

        // The serialized form should be exactly the same as if we serialized MyAttrs directly
        let expected = r#"{"author":"John Doe","version":"1.0.0"}"#;
        assert_eq!(json, expected);

        // Verify it deserializes back correctly
        let deserialized: Attrs<MyAttrs> = serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(deserialized, attrs);
    }

    #[test]
    fn test_none_serializes_as_empty_map() {
        // This test demonstrates that Attrs::None always serializes as {}
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct MyAttrs {
            field: String,
        }

        let attrs: Attrs<MyAttrs> = Attrs::None;
        let json = serde_json::to_string(&attrs).expect("Failed to serialize");

        assert_eq!(json, "{}");

        // Verify {} deserializes back to None
        let deserialized: Attrs<MyAttrs> = serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(deserialized, Attrs::None);
    }
}

//! Serde implementations for Morphir IR types.
//!
//! Serialization uses V4 object wrapper format:
//! - `{ "Variable": { "name": "a" } }`
//! - `{ "Reference": { "fqname": "morphir/sdk:basics#int" } }`
//! - `{ "Tuple": { "elements": [...] } }`
//!
//! Deserialization accepts both V4 and Classic formats for backward compatibility:
//! - V4 object: `{ "Variable": { "name": "a" } }`
//! - Classic array: `["Variable", attrs, name]`
//! - V4 shorthand: `"a"` (variable) or `"morphir/sdk:basics#int"` (reference)

use serde::de::{self, DeserializeOwned, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::marker::PhantomData;

use super::literal::Literal;
use super::pattern::Pattern;
use super::serde_v4;
use super::type_expr::{Field, Type};
use crate::naming::{FQName, Name};

// =============================================================================
// Type<A> Serialization
// =============================================================================

impl<A: Clone + Serialize> Serialize for Type<A> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Delegate to V4 object wrapper format
        serde_v4::serialize_type(self, serializer)
    }
}

impl<'de, A: Clone + Default + DeserializeOwned> Deserialize<'de> for Type<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Use deserialize_any to accept V4 objects, Classic arrays, and string shorthand
        deserializer.deserialize_any(TypeVisitor(PhantomData))
    }
}

struct TypeVisitor<A>(PhantomData<A>);

impl<'de, A: Clone + Default + DeserializeOwned> Visitor<'de> for TypeVisitor<A> {
    type Value = Type<A>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(
            "V4 object { \"Variable\": { \"name\": \"a\" } }, \
             Classic array [\"Variable\", attrs, name], \
             or string shorthand \"a\"",
        )
    }

    /// V4 object wrapper format: { "Variable": { "name": "a" } }
    fn visit_map<M>(self, mut map: M) -> Result<Type<A>, M::Error>
    where
        M: MapAccess<'de>,
    {
        use indexmap::IndexMap;

        let (tag, value): (String, serde_json::Value) = map
            .next_entry()?
            .ok_or_else(|| de::Error::custom("expected object wrapper with single key"))?;

        match tag.as_str() {
            "Variable" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content<A> {
                    name: String,
                    attrs: Option<A>,
                }
                let content: Content<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let name = Name::from(content.name.as_str());
                let attrs = content.attrs.unwrap_or_default();
                Ok(Type::Variable(attrs, name))
            }
            "Reference" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "A: Clone + Default + DeserializeOwned"))]
                struct Content<A: Clone> {
                    fqname: String,
                    args: Option<Vec<Type<A>>>,
                    attrs: Option<A>,
                }
                let content: Content<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let fqname = FQName::from_canonical_string(&content.fqname)
                    .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
                let attrs = content.attrs.unwrap_or_default();
                let args = content.args.unwrap_or_default();
                Ok(Type::Reference(attrs, fqname, args))
            }
            "Tuple" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "A: Clone + Default + DeserializeOwned"))]
                struct Content<A: Clone> {
                    elements: Vec<Type<A>>,
                    attrs: Option<A>,
                }
                let content: Content<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Type::Tuple(attrs, content.elements))
            }
            "Record" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "A: Clone + Default + DeserializeOwned"))]
                struct Content<A: Clone> {
                    fields: IndexMap<String, Type<A>>,
                    attrs: Option<A>,
                }
                let content: Content<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let fields = content
                    .fields
                    .into_iter()
                    .map(|(name, tpe)| Field {
                        name: Name::from(name.as_str()),
                        tpe,
                    })
                    .collect();
                Ok(Type::Record(attrs, fields))
            }
            "ExtensibleRecord" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "A: Clone + Default + DeserializeOwned"))]
                struct Content<A: Clone> {
                    variable: String,
                    fields: IndexMap<String, Type<A>>,
                    attrs: Option<A>,
                }
                let content: Content<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let variable = Name::from(content.variable.as_str());
                let fields = content
                    .fields
                    .into_iter()
                    .map(|(name, tpe)| Field {
                        name: Name::from(name.as_str()),
                        tpe,
                    })
                    .collect();
                Ok(Type::ExtensibleRecord(attrs, variable, fields))
            }
            "Function" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "A: Clone + Default + DeserializeOwned"))]
                struct Content<A: Clone> {
                    arg: Type<A>,
                    result: Type<A>,
                    attrs: Option<A>,
                }
                let content: Content<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Type::Function(
                    attrs,
                    Box::new(content.arg),
                    Box::new(content.result),
                ))
            }
            "Unit" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content<A> {
                    attrs: Option<A>,
                }
                let content: Content<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Type::Unit(attrs))
            }
            _ => Err(de::Error::unknown_variant(
                &tag,
                &[
                    "Variable",
                    "Reference",
                    "Tuple",
                    "Record",
                    "ExtensibleRecord",
                    "Function",
                    "Unit",
                ],
            )),
        }
    }

    /// Classic tagged array format: ["Variable", attrs, name]
    fn visit_seq<V>(self, mut seq: V) -> Result<Type<A>, V::Error>
    where
        V: SeqAccess<'de>,
    {
        let tag: String = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;

        match tag.as_str() {
            "Variable" | "variable" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let name: Name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Type::Variable(attrs, name))
            }
            "Reference" | "reference" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let fqname: FQName = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let params: Vec<Type<A>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Type::Reference(attrs, fqname, params))
            }
            "Tuple" | "tuple" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let elements: Vec<Type<A>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Type::Tuple(attrs, elements))
            }
            "Record" | "record" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let fields: Vec<Field<A>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Type::Record(attrs, fields))
            }
            "ExtensibleRecord" | "extensibleRecord" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let var: Name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let fields: Vec<Field<A>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Type::ExtensibleRecord(attrs, var, fields))
            }
            "Function" | "function" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let arg: Type<A> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let result: Type<A> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Type::Function(attrs, Box::new(arg), Box::new(result)))
            }
            "Unit" | "unit" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Type::Unit(attrs))
            }
            _ => Err(de::Error::unknown_variant(
                &tag,
                &[
                    "Variable",
                    "Reference",
                    "Tuple",
                    "Record",
                    "ExtensibleRecord",
                    "Function",
                    "Unit",
                ],
            )),
        }
    }

    /// V4 string shorthand: "a" for Variable, "morphir/sdk:basics#int" for Reference
    fn visit_str<E>(self, v: &str) -> Result<Type<A>, E>
    where
        E: de::Error,
    {
        if v.contains(':') && v.contains('#') {
            // FQName shorthand for Reference
            let fqname = FQName::from_canonical_string(v)
                .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
            Ok(Type::Reference(A::default(), fqname, vec![]))
        } else {
            // Variable shorthand
            let name = Name::from(v);
            Ok(Type::Variable(A::default(), name))
        }
    }
}

// =============================================================================
// Field<A> Serialization
// =============================================================================

impl<A: Clone + Serialize> Serialize for Field<A> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Field", 2)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("tpe", &self.tpe)?;
        state.end()
    }
}

impl<'de, A: Clone + Default + DeserializeOwned> Deserialize<'de> for Field<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum FieldName {
            Name,
            Tpe,
        }

        struct FieldVisitor<A>(PhantomData<A>);

        impl<'de, A: Clone + Default + DeserializeOwned> Visitor<'de> for FieldVisitor<A> {
            type Value = Field<A>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a field object with name and tpe")
            }

            fn visit_map<V>(self, mut map: V) -> Result<Field<A>, V::Error>
            where
                V: de::MapAccess<'de>,
            {
                let mut name = None;
                let mut tpe = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        FieldName::Name => {
                            if name.is_some() {
                                return Err(de::Error::duplicate_field("name"));
                            }
                            name = Some(map.next_value()?);
                        }
                        FieldName::Tpe => {
                            if tpe.is_some() {
                                return Err(de::Error::duplicate_field("tpe"));
                            }
                            tpe = Some(map.next_value()?);
                        }
                    }
                }

                let name = name.ok_or_else(|| de::Error::missing_field("name"))?;
                let tpe = tpe.ok_or_else(|| de::Error::missing_field("tpe"))?;
                Ok(Field { name, tpe })
            }
        }

        deserializer.deserialize_struct("Field", &["name", "tpe"], FieldVisitor(PhantomData))
    }
}

// =============================================================================
// Pattern<A> Serialization
// =============================================================================

impl<A: Clone + Serialize> Serialize for Pattern<A> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Delegate to V4 object wrapper format
        serde_v4::serialize_pattern(self, serializer)
    }
}

impl<'de, A: Clone + Default + DeserializeOwned> Deserialize<'de> for Pattern<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Use deserialize_any to accept V4 objects and Classic arrays
        deserializer.deserialize_any(PatternVisitor(PhantomData))
    }
}

struct PatternVisitor<A>(PhantomData<A>);

impl<'de, A: Clone + Default + DeserializeOwned> Visitor<'de> for PatternVisitor<A> {
    type Value = Pattern<A>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(
            "V4 object { \"WildcardPattern\": {} } or Classic array [\"WildcardPattern\", attrs]",
        )
    }

    /// V4 object wrapper format: { "WildcardPattern": {} }
    fn visit_map<M>(self, mut map: M) -> Result<Pattern<A>, M::Error>
    where
        M: MapAccess<'de>,
    {
        let (tag, value): (String, serde_json::Value) = map
            .next_entry()?
            .ok_or_else(|| de::Error::custom("expected object wrapper with single key"))?;

        match tag.as_str() {
            "WildcardPattern" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content<A> {
                    attrs: Option<A>,
                }
                let content: Content<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Pattern::WildcardPattern(attrs))
            }
            "AsPattern" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "A: Clone + Default + DeserializeOwned"))]
                struct Content<A: Clone> {
                    pattern: Pattern<A>,
                    name: String,
                    attrs: Option<A>,
                }
                let content: Content<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let name = Name::from(content.name.as_str());
                Ok(Pattern::AsPattern(attrs, Box::new(content.pattern), name))
            }
            "TuplePattern" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "A: Clone + Default + DeserializeOwned"))]
                struct Content<A: Clone> {
                    elements: Vec<Pattern<A>>,
                    attrs: Option<A>,
                }
                let content: Content<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Pattern::TuplePattern(attrs, content.elements))
            }
            "ConstructorPattern" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "A: Clone + Default + DeserializeOwned"))]
                struct Content<A: Clone> {
                    fqname: String,
                    args: Vec<Pattern<A>>,
                    attrs: Option<A>,
                }
                let content: Content<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let fqname = FQName::from_canonical_string(&content.fqname)
                    .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
                Ok(Pattern::ConstructorPattern(attrs, fqname, content.args))
            }
            "EmptyListPattern" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content<A> {
                    attrs: Option<A>,
                }
                let content: Content<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Pattern::EmptyListPattern(attrs))
            }
            "HeadTailPattern" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "A: Clone + Default + DeserializeOwned"))]
                struct Content<A: Clone> {
                    head: Pattern<A>,
                    tail: Pattern<A>,
                    attrs: Option<A>,
                }
                let content: Content<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Pattern::HeadTailPattern(
                    attrs,
                    Box::new(content.head),
                    Box::new(content.tail),
                ))
            }
            "LiteralPattern" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content<A> {
                    literal: Literal,
                    attrs: Option<A>,
                }
                let content: Content<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Pattern::LiteralPattern(attrs, content.literal))
            }
            "UnitPattern" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content<A> {
                    attrs: Option<A>,
                }
                let content: Content<A> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Pattern::UnitPattern(attrs))
            }
            _ => Err(de::Error::unknown_variant(
                &tag,
                &[
                    "WildcardPattern",
                    "AsPattern",
                    "TuplePattern",
                    "ConstructorPattern",
                    "EmptyListPattern",
                    "HeadTailPattern",
                    "LiteralPattern",
                    "UnitPattern",
                ],
            )),
        }
    }

    /// Classic tagged array format: ["WildcardPattern", attrs]
    fn visit_seq<V>(self, mut seq: V) -> Result<Pattern<A>, V::Error>
    where
        V: SeqAccess<'de>,
    {
        let tag: String = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;

        match tag.as_str() {
            "WildcardPattern" | "wildcardPattern" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Pattern::WildcardPattern(attrs))
            }
            "AsPattern" | "asPattern" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let pattern: Pattern<A> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let name: Name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Pattern::AsPattern(attrs, Box::new(pattern), name))
            }
            "TuplePattern" | "tuplePattern" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let elements: Vec<Pattern<A>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Pattern::TuplePattern(attrs, elements))
            }
            "ConstructorPattern" | "constructorPattern" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let name: FQName = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let args: Vec<Pattern<A>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Pattern::ConstructorPattern(attrs, name, args))
            }
            "EmptyListPattern" | "emptyListPattern" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Pattern::EmptyListPattern(attrs))
            }
            "HeadTailPattern" | "headTailPattern" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let head: Pattern<A> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let tail: Pattern<A> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Pattern::HeadTailPattern(
                    attrs,
                    Box::new(head),
                    Box::new(tail),
                ))
            }
            "LiteralPattern" | "literalPattern" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let lit: Literal = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Pattern::LiteralPattern(attrs, lit))
            }
            "UnitPattern" | "unitPattern" => {
                let attrs: A = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Pattern::UnitPattern(attrs))
            }
            _ => Err(de::Error::unknown_variant(
                &tag,
                &[
                    "WildcardPattern",
                    "AsPattern",
                    "TuplePattern",
                    "ConstructorPattern",
                    "EmptyListPattern",
                    "HeadTailPattern",
                    "LiteralPattern",
                    "UnitPattern",
                ],
            )),
        }
    }
}

// =============================================================================
// Value<TA, VA> Serialization
// =============================================================================

use super::type_def::ConstructorArg;
use super::value_expr::{
    HoleReason, InputType, LetBinding, NativeHint, NativeInfo, PatternCase, RecordFieldEntry,
    Value, ValueBody, ValueDefinition,
};

impl<TA: Clone + Serialize, VA: Clone + Serialize> Serialize for Value<TA, VA> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Delegate to V4 object wrapper format
        serde_v4::serialize_value(self, serializer)
    }
}

// Serialize HoleReason
impl Serialize for HoleReason {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            HoleReason::UnresolvedReference { target } => {
                use serde::ser::SerializeStruct;
                let mut state = serializer.serialize_struct("HoleReason", 2)?;
                state.serialize_field("kind", "UnresolvedReference")?;
                state.serialize_field("target", target)?;
                state.end()
            }
            HoleReason::DeletedDuringRefactor { tx_id } => {
                use serde::ser::SerializeStruct;
                let mut state = serializer.serialize_struct("HoleReason", 2)?;
                state.serialize_field("kind", "DeletedDuringRefactor")?;
                state.serialize_field("tx-id", tx_id)?;
                state.end()
            }
            HoleReason::TypeMismatch { expected, found } => {
                use serde::ser::SerializeStruct;
                let mut state = serializer.serialize_struct("HoleReason", 3)?;
                state.serialize_field("kind", "TypeMismatch")?;
                state.serialize_field("expected", expected)?;
                state.serialize_field("found", found)?;
                state.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for HoleReason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Accept either a string (legacy) or an object
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(s) => match s.as_str() {
                // Legacy simple strings - provide default values
                "DeletedDuringRefactor" => Ok(HoleReason::DeletedDuringRefactor {
                    tx_id: "unknown".to_string(),
                }),
                "TypeMismatch" => Ok(HoleReason::TypeMismatch {
                    expected: "unknown".to_string(),
                    found: "unknown".to_string(),
                }),
                _ => Err(de::Error::unknown_variant(
                    &s,
                    &["DeletedDuringRefactor", "TypeMismatch"],
                )),
            },
            serde_json::Value::Object(map) => {
                let kind = map
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| de::Error::missing_field("kind"))?;
                match kind {
                    "UnresolvedReference" => {
                        let target: FQName = serde_json::from_value(
                            map.get("target")
                                .cloned()
                                .ok_or_else(|| de::Error::missing_field("target"))?,
                        )
                        .map_err(de::Error::custom)?;
                        Ok(HoleReason::UnresolvedReference { target })
                    }
                    "DeletedDuringRefactor" => {
                        let tx_id = map
                            .get("tx-id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        Ok(HoleReason::DeletedDuringRefactor { tx_id })
                    }
                    "TypeMismatch" => {
                        let expected = map
                            .get("expected")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let found = map
                            .get("found")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        Ok(HoleReason::TypeMismatch { expected, found })
                    }
                    _ => Err(de::Error::unknown_variant(
                        kind,
                        &[
                            "UnresolvedReference",
                            "DeletedDuringRefactor",
                            "TypeMismatch",
                        ],
                    )),
                }
            }
            _ => Err(de::Error::custom(
                "expected string or object for HoleReason",
            )),
        }
    }
}

// Serialize NativeHint
impl Serialize for NativeHint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            NativeHint::Arithmetic => serializer.serialize_str("Arithmetic"),
            NativeHint::Comparison => serializer.serialize_str("Comparison"),
            NativeHint::StringOp => serializer.serialize_str("StringOp"),
            NativeHint::CollectionOp => serializer.serialize_str("CollectionOp"),
            NativeHint::PlatformSpecific { platform } => {
                use serde::ser::SerializeStruct;
                let mut state = serializer.serialize_struct("NativeHint", 2)?;
                state.serialize_field("kind", "PlatformSpecific")?;
                state.serialize_field("platform", platform)?;
                state.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for NativeHint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(s) => match s.as_str() {
                "Arithmetic" => Ok(NativeHint::Arithmetic),
                "Comparison" => Ok(NativeHint::Comparison),
                "StringOp" => Ok(NativeHint::StringOp),
                "CollectionOp" => Ok(NativeHint::CollectionOp),
                "PlatformSpecific" => Ok(NativeHint::PlatformSpecific {
                    platform: "unknown".to_string(),
                }),
                _ => Err(de::Error::unknown_variant(
                    &s,
                    &[
                        "Arithmetic",
                        "Comparison",
                        "StringOp",
                        "CollectionOp",
                        "PlatformSpecific",
                    ],
                )),
            },
            serde_json::Value::Object(map) => {
                let kind = map
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| de::Error::missing_field("kind"))?;
                match kind {
                    "Arithmetic" => Ok(NativeHint::Arithmetic),
                    "Comparison" => Ok(NativeHint::Comparison),
                    "StringOp" => Ok(NativeHint::StringOp),
                    "CollectionOp" => Ok(NativeHint::CollectionOp),
                    "PlatformSpecific" => {
                        let platform = map
                            .get("platform")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        Ok(NativeHint::PlatformSpecific { platform })
                    }
                    _ => Err(de::Error::unknown_variant(
                        kind,
                        &[
                            "Arithmetic",
                            "Comparison",
                            "StringOp",
                            "CollectionOp",
                            "PlatformSpecific",
                        ],
                    )),
                }
            }
            _ => Err(de::Error::custom(
                "expected string or object for NativeHint",
            )),
        }
    }
}

// Serialize NativeInfo
impl Serialize for NativeInfo {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("NativeInfo", 2)?;
        state.serialize_field("hint", &self.hint)?;
        state.serialize_field("description", &self.description)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for NativeInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct NativeInfoHelper {
            hint: NativeHint,
            description: Option<String>,
        }
        let helper = NativeInfoHelper::deserialize(deserializer)?;
        Ok(NativeInfo {
            hint: helper.hint,
            description: helper.description,
        })
    }
}

// Serialize ValueDefinition
impl<TA: Clone + Serialize, VA: Clone + Serialize> Serialize for ValueDefinition<TA, VA> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ValueDefinition", 3)?;
        state.serialize_field("inputTypes", &self.input_types)?;
        state.serialize_field("outputType", &self.output_type)?;
        state.serialize_field("body", &self.body)?;
        state.end()
    }
}

// Serialize ValueBody
impl<TA: Clone + Serialize, VA: Clone + Serialize> Serialize for ValueBody<TA, VA> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            ValueBody::Expression(val) => {
                use serde::ser::SerializeStruct;
                let mut state = serializer.serialize_struct("ValueBody", 2)?;
                state.serialize_field("kind", "Expression")?;
                state.serialize_field("value", val)?;
                state.end()
            }
            ValueBody::Native(info) => {
                use serde::ser::SerializeStruct;
                let mut state = serializer.serialize_struct("ValueBody", 2)?;
                state.serialize_field("kind", "Native")?;
                state.serialize_field("info", info)?;
                state.end()
            }
            ValueBody::External {
                external_name,
                target_platform,
            } => {
                use serde::ser::SerializeStruct;
                let mut state = serializer.serialize_struct("ValueBody", 3)?;
                state.serialize_field("kind", "External")?;
                state.serialize_field("externalName", external_name)?;
                state.serialize_field("targetPlatform", target_platform)?;
                state.end()
            }
            ValueBody::Incomplete(reason) => {
                use serde::ser::SerializeStruct;
                let mut state = serializer.serialize_struct("ValueBody", 2)?;
                state.serialize_field("kind", "Incomplete")?;
                state.serialize_field("reason", reason)?;
                state.end()
            }
        }
    }
}

// =============================================================================
// Tuple Struct Serialization (InputType, RecordFieldEntry, PatternCase, LetBinding, ConstructorArg)
// =============================================================================

// InputType<TA, VA>(Name, VA, Type<TA>) - serialize as [name, attrs, type]
impl<TA: Clone + Serialize, VA: Clone + Serialize> Serialize for InputType<TA, VA> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(3))?;
        seq.serialize_element(&self.0)?;
        seq.serialize_element(&self.1)?;
        seq.serialize_element(&self.2)?;
        seq.end()
    }
}

impl<'de, TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned> Deserialize<'de>
    for InputType<TA, VA>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct InputTypeVisitor<TA, VA>(PhantomData<(TA, VA)>);

        impl<'de, TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned> Visitor<'de>
            for InputTypeVisitor<TA, VA>
        {
            type Value = InputType<TA, VA>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a tuple [name, attrs, type]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<InputType<TA, VA>, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let attrs = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let tpe = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(InputType(name, attrs, tpe))
            }
        }

        deserializer.deserialize_seq(InputTypeVisitor(PhantomData))
    }
}

// RecordFieldEntry<TA, VA>(Name, Value<TA, VA>) - serialize as [name, value]
impl<TA: Clone + Serialize, VA: Clone + Serialize> Serialize for RecordFieldEntry<TA, VA> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(2))?;
        seq.serialize_element(&self.0)?;
        seq.serialize_element(&self.1)?;
        seq.end()
    }
}

impl<'de, TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned> Deserialize<'de>
    for RecordFieldEntry<TA, VA>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RecordFieldEntryVisitor<TA, VA>(PhantomData<(TA, VA)>);

        impl<'de, TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned> Visitor<'de>
            for RecordFieldEntryVisitor<TA, VA>
        {
            type Value = RecordFieldEntry<TA, VA>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a tuple [name, value]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<RecordFieldEntry<TA, VA>, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let value = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(RecordFieldEntry(name, value))
            }
        }

        deserializer.deserialize_seq(RecordFieldEntryVisitor(PhantomData))
    }
}

// PatternCase<TA, VA>(Pattern<VA>, Value<TA, VA>) - serialize as [pattern, body]
impl<TA: Clone + Serialize, VA: Clone + Serialize> Serialize for PatternCase<TA, VA> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(2))?;
        seq.serialize_element(&self.0)?;
        seq.serialize_element(&self.1)?;
        seq.end()
    }
}

impl<'de, TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned> Deserialize<'de>
    for PatternCase<TA, VA>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PatternCaseVisitor<TA, VA>(PhantomData<(TA, VA)>);

        impl<'de, TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned> Visitor<'de>
            for PatternCaseVisitor<TA, VA>
        {
            type Value = PatternCase<TA, VA>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a tuple [pattern, body]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<PatternCase<TA, VA>, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let pattern = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let body = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(PatternCase(pattern, body))
            }
        }

        deserializer.deserialize_seq(PatternCaseVisitor(PhantomData))
    }
}

// LetBinding<TA, VA>(Name, ValueDefinition<TA, VA>) - serialize as [name, definition]
impl<TA: Clone + Serialize, VA: Clone + Serialize> Serialize for LetBinding<TA, VA> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(2))?;
        seq.serialize_element(&self.0)?;
        seq.serialize_element(&self.1)?;
        seq.end()
    }
}

impl<'de, TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned> Deserialize<'de>
    for LetBinding<TA, VA>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LetBindingVisitor<TA, VA>(PhantomData<(TA, VA)>);

        impl<'de, TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned> Visitor<'de>
            for LetBindingVisitor<TA, VA>
        {
            type Value = LetBinding<TA, VA>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a tuple [name, definition]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<LetBinding<TA, VA>, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let definition = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(LetBinding(name, definition))
            }
        }

        deserializer.deserialize_seq(LetBindingVisitor(PhantomData))
    }
}

// ConstructorArg<A>(Name, Type<A>) - serialize as [name, type]
impl<A: Clone + Serialize> Serialize for ConstructorArg<A> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(2))?;
        seq.serialize_element(&self.0)?;
        seq.serialize_element(&self.1)?;
        seq.end()
    }
}

impl<'de, A: Clone + Default + DeserializeOwned> Deserialize<'de> for ConstructorArg<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ConstructorArgVisitor<A>(PhantomData<A>);

        impl<'de, A: Clone + Default + DeserializeOwned> Visitor<'de> for ConstructorArgVisitor<A> {
            type Value = ConstructorArg<A>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a tuple [name, type]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<ConstructorArg<A>, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let tpe = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(ConstructorArg(name, tpe))
            }
        }

        deserializer.deserialize_seq(ConstructorArgVisitor(PhantomData))
    }
}

// Deserialize for ValueDefinition
impl<'de, TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned> Deserialize<'de>
    for ValueDefinition<TA, VA>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "camelCase")]
        enum FieldName {
            InputTypes,
            OutputType,
            Body,
        }

        struct ValueDefinitionVisitor<TA, VA>(PhantomData<(TA, VA)>);

        impl<'de, TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned> Visitor<'de>
            for ValueDefinitionVisitor<TA, VA>
        {
            type Value = ValueDefinition<TA, VA>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a value definition with inputTypes, outputType, and body")
            }

            fn visit_map<V>(self, mut map: V) -> Result<ValueDefinition<TA, VA>, V::Error>
            where
                V: de::MapAccess<'de>,
            {
                let mut input_types = None;
                let mut output_type = None;
                let mut body = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        FieldName::InputTypes => {
                            if input_types.is_some() {
                                return Err(de::Error::duplicate_field("inputTypes"));
                            }
                            input_types = Some(map.next_value()?);
                        }
                        FieldName::OutputType => {
                            if output_type.is_some() {
                                return Err(de::Error::duplicate_field("outputType"));
                            }
                            output_type = Some(map.next_value()?);
                        }
                        FieldName::Body => {
                            if body.is_some() {
                                return Err(de::Error::duplicate_field("body"));
                            }
                            body = Some(map.next_value()?);
                        }
                    }
                }

                let input_types =
                    input_types.ok_or_else(|| de::Error::missing_field("inputTypes"))?;
                let output_type =
                    output_type.ok_or_else(|| de::Error::missing_field("outputType"))?;
                let body = body.ok_or_else(|| de::Error::missing_field("body"))?;
                Ok(ValueDefinition {
                    input_types,
                    output_type,
                    body,
                })
            }
        }

        deserializer.deserialize_struct(
            "ValueDefinition",
            &["inputTypes", "outputType", "body"],
            ValueDefinitionVisitor(PhantomData),
        )
    }
}

// Deserialize for ValueBody
impl<'de, TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned> Deserialize<'de>
    for ValueBody<TA, VA>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "camelCase")]
        enum FieldName {
            Kind,
            Value,
            Info,
            ExternalName,
            TargetPlatform,
            Reason,
        }

        struct ValueBodyVisitor<TA, VA>(PhantomData<(TA, VA)>);

        impl<'de, TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned> Visitor<'de>
            for ValueBodyVisitor<TA, VA>
        {
            type Value = ValueBody<TA, VA>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a value body with kind field")
            }

            fn visit_map<V>(self, mut map: V) -> Result<ValueBody<TA, VA>, V::Error>
            where
                V: de::MapAccess<'de>,
            {
                let mut kind: Option<String> = None;
                let mut value: Option<Value<TA, VA>> = None;
                let mut info: Option<NativeInfo> = None;
                let mut external_name: Option<String> = None;
                let mut target_platform: Option<String> = None;
                let mut reason: Option<HoleReason> = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        FieldName::Kind => {
                            if kind.is_some() {
                                return Err(de::Error::duplicate_field("kind"));
                            }
                            kind = Some(map.next_value()?);
                        }
                        FieldName::Value => {
                            if value.is_some() {
                                return Err(de::Error::duplicate_field("value"));
                            }
                            value = Some(map.next_value()?);
                        }
                        FieldName::Info => {
                            if info.is_some() {
                                return Err(de::Error::duplicate_field("info"));
                            }
                            info = Some(map.next_value()?);
                        }
                        FieldName::ExternalName => {
                            if external_name.is_some() {
                                return Err(de::Error::duplicate_field("externalName"));
                            }
                            external_name = Some(map.next_value()?);
                        }
                        FieldName::TargetPlatform => {
                            if target_platform.is_some() {
                                return Err(de::Error::duplicate_field("targetPlatform"));
                            }
                            target_platform = Some(map.next_value()?);
                        }
                        FieldName::Reason => {
                            if reason.is_some() {
                                return Err(de::Error::duplicate_field("reason"));
                            }
                            reason = Some(map.next_value()?);
                        }
                    }
                }

                let kind = kind.ok_or_else(|| de::Error::missing_field("kind"))?;
                match kind.as_str() {
                    "Expression" => {
                        let val = value.ok_or_else(|| de::Error::missing_field("value"))?;
                        Ok(ValueBody::Expression(val))
                    }
                    "Native" => {
                        let info = info.ok_or_else(|| de::Error::missing_field("info"))?;
                        Ok(ValueBody::Native(info))
                    }
                    "External" => {
                        let external_name = external_name
                            .ok_or_else(|| de::Error::missing_field("externalName"))?;
                        let target_platform = target_platform
                            .ok_or_else(|| de::Error::missing_field("targetPlatform"))?;
                        Ok(ValueBody::External {
                            external_name,
                            target_platform,
                        })
                    }
                    "Incomplete" => {
                        let reason = reason.ok_or_else(|| de::Error::missing_field("reason"))?;
                        Ok(ValueBody::Incomplete(reason))
                    }
                    _ => Err(de::Error::unknown_variant(
                        &kind,
                        &["Expression", "Native", "External", "Incomplete"],
                    )),
                }
            }
        }

        deserializer.deserialize_struct(
            "ValueBody",
            &[
                "kind",
                "value",
                "info",
                "externalName",
                "targetPlatform",
                "reason",
            ],
            ValueBodyVisitor(PhantomData),
        )
    }
}

// Deserialize for Value (complex, needs visitor)
impl<'de, TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned> Deserialize<'de>
    for Value<TA, VA>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Use deserialize_any to accept V4 objects and Classic arrays
        deserializer.deserialize_any(ValueVisitor(PhantomData))
    }
}

struct ValueVisitor<TA, VA>(PhantomData<(TA, VA)>);

impl<'de, TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned> Visitor<'de>
    for ValueVisitor<TA, VA>
{
    type Value = Value<TA, VA>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(
            "V4 object { \"Variable\": { \"name\": \"x\" } } or Classic array [\"Literal\", attrs, lit]",
        )
    }

    /// V4 object wrapper format: { "Variable": { "name": "x" } }
    fn visit_map<M>(self, mut map: M) -> Result<Value<TA, VA>, M::Error>
    where
        M: MapAccess<'de>,
    {
        use super::value_expr::RecordFieldEntry;
        use indexmap::IndexMap;

        let (tag, value): (String, serde_json::Value) = map
            .next_entry()?
            .ok_or_else(|| de::Error::custom("expected object wrapper with single key"))?;

        match tag.as_str() {
            "Literal" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content<VA> {
                    literal: Literal,
                    attrs: Option<VA>,
                }
                let content: Content<VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::Literal(attrs, content.literal))
            }
            "Constructor" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content<VA> {
                    fqname: String,
                    attrs: Option<VA>,
                }
                let content: Content<VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let fqname = FQName::from_canonical_string(&content.fqname)
                    .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
                Ok(Value::Constructor(attrs, fqname))
            }
            "Tuple" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"))]
                struct Content<TA: Clone, VA: Clone> {
                    elements: Vec<Value<TA, VA>>,
                    attrs: Option<VA>,
                }
                let content: Content<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::Tuple(attrs, content.elements))
            }
            "List" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"))]
                struct Content<TA: Clone, VA: Clone> {
                    items: Vec<Value<TA, VA>>,
                    attrs: Option<VA>,
                }
                let content: Content<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::List(attrs, content.items))
            }
            "Record" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"))]
                struct Content<TA: Clone, VA: Clone> {
                    fields: IndexMap<String, Value<TA, VA>>,
                    attrs: Option<VA>,
                }
                let content: Content<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let fields = content
                    .fields
                    .into_iter()
                    .map(|(name, val)| RecordFieldEntry(Name::from(name.as_str()), val))
                    .collect();
                Ok(Value::Record(attrs, fields))
            }
            "Variable" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content<VA> {
                    name: String,
                    attrs: Option<VA>,
                }
                let content: Content<VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let name = Name::from(content.name.as_str());
                Ok(Value::Variable(attrs, name))
            }
            "Reference" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content<VA> {
                    fqname: String,
                    attrs: Option<VA>,
                }
                let content: Content<VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let fqname = FQName::from_canonical_string(&content.fqname)
                    .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
                Ok(Value::Reference(attrs, fqname))
            }
            "Field" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"))]
                struct Content<TA: Clone, VA: Clone> {
                    value: Value<TA, VA>,
                    name: String,
                    attrs: Option<VA>,
                }
                let content: Content<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let name = Name::from(content.name.as_str());
                Ok(Value::Field(attrs, Box::new(content.value), name))
            }
            "FieldFunction" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content<VA> {
                    name: String,
                    attrs: Option<VA>,
                }
                let content: Content<VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let name = Name::from(content.name.as_str());
                Ok(Value::FieldFunction(attrs, name))
            }
            "Apply" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"))]
                struct Content<TA: Clone, VA: Clone> {
                    function: Value<TA, VA>,
                    argument: Value<TA, VA>,
                    attrs: Option<VA>,
                }
                let content: Content<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::Apply(
                    attrs,
                    Box::new(content.function),
                    Box::new(content.argument),
                ))
            }
            "Lambda" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"))]
                struct Content<TA: Clone, VA: Clone> {
                    #[serde(rename = "argumentPattern")]
                    argument_pattern: Pattern<VA>,
                    body: Value<TA, VA>,
                    attrs: Option<VA>,
                }
                let content: Content<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::Lambda(
                    attrs,
                    content.argument_pattern,
                    Box::new(content.body),
                ))
            }
            "LetDefinition" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"))]
                struct Content<TA: Clone, VA: Clone> {
                    name: String,
                    definition: ValueDefinition<TA, VA>,
                    body: Value<TA, VA>,
                    attrs: Option<VA>,
                }
                let content: Content<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let name = Name::from(content.name.as_str());
                Ok(Value::LetDefinition(
                    attrs,
                    name,
                    Box::new(content.definition),
                    Box::new(content.body),
                ))
            }
            "LetRecursion" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"))]
                struct Content<TA: Clone, VA: Clone> {
                    definitions: Vec<LetBinding<TA, VA>>,
                    body: Value<TA, VA>,
                    attrs: Option<VA>,
                }
                let content: Content<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::LetRecursion(
                    attrs,
                    content.definitions,
                    Box::new(content.body),
                ))
            }
            "Destructure" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"))]
                struct Content<TA: Clone, VA: Clone> {
                    pattern: Pattern<VA>,
                    value: Value<TA, VA>,
                    body: Value<TA, VA>,
                    attrs: Option<VA>,
                }
                let content: Content<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::Destructure(
                    attrs,
                    content.pattern,
                    Box::new(content.value),
                    Box::new(content.body),
                ))
            }
            "IfThenElse" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"))]
                struct Content<TA: Clone, VA: Clone> {
                    condition: Value<TA, VA>,
                    #[serde(rename = "thenBranch")]
                    then_branch: Value<TA, VA>,
                    #[serde(rename = "elseBranch")]
                    else_branch: Value<TA, VA>,
                    attrs: Option<VA>,
                }
                let content: Content<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::IfThenElse(
                    attrs,
                    Box::new(content.condition),
                    Box::new(content.then_branch),
                    Box::new(content.else_branch),
                ))
            }
            "PatternMatch" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"))]
                struct Content<TA: Clone, VA: Clone> {
                    value: Value<TA, VA>,
                    cases: Vec<PatternCase<TA, VA>>,
                    attrs: Option<VA>,
                }
                let content: Content<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::PatternMatch(
                    attrs,
                    Box::new(content.value),
                    content.cases,
                ))
            }
            "UpdateRecord" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"))]
                struct Content<TA: Clone, VA: Clone> {
                    value: Value<TA, VA>,
                    fields: IndexMap<String, Value<TA, VA>>,
                    attrs: Option<VA>,
                }
                let content: Content<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let updates = content
                    .fields
                    .into_iter()
                    .map(|(name, val)| RecordFieldEntry(Name::from(name.as_str()), val))
                    .collect();
                Ok(Value::UpdateRecord(attrs, Box::new(content.value), updates))
            }
            "Unit" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content<VA> {
                    attrs: Option<VA>,
                }
                let content: Content<VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::Unit(attrs))
            }
            "Hole" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                #[serde(bound(deserialize = "TA: Clone + Default + DeserializeOwned, VA: Clone + Default + DeserializeOwned"))]
                struct Content<TA: Clone, VA: Clone> {
                    reason: HoleReason,
                    #[serde(rename = "expectedType")]
                    expected_type: Option<Type<TA>>,
                    attrs: Option<VA>,
                }
                let content: Content<TA, VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::Hole(
                    attrs,
                    content.reason,
                    content.expected_type.map(Box::new),
                ))
            }
            "Native" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content<VA> {
                    fqname: String,
                    info: NativeInfo,
                    attrs: Option<VA>,
                }
                let content: Content<VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                let fqname = FQName::from_canonical_string(&content.fqname)
                    .map_err(|e| de::Error::custom(format!("invalid FQName: {}", e)))?;
                Ok(Value::Native(attrs, fqname, content.info))
            }
            "External" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Content<VA> {
                    #[serde(rename = "externalName")]
                    external_name: String,
                    #[serde(rename = "targetPlatform")]
                    target_platform: String,
                    attrs: Option<VA>,
                }
                let content: Content<VA> =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                let attrs = content.attrs.unwrap_or_default();
                Ok(Value::External(
                    attrs,
                    content.external_name,
                    content.target_platform,
                ))
            }
            _ => Err(de::Error::unknown_variant(
                &tag,
                &[
                    "Literal",
                    "Constructor",
                    "Tuple",
                    "List",
                    "Record",
                    "Variable",
                    "Reference",
                    "Field",
                    "FieldFunction",
                    "Apply",
                    "Lambda",
                    "LetDefinition",
                    "LetRecursion",
                    "Destructure",
                    "IfThenElse",
                    "PatternMatch",
                    "UpdateRecord",
                    "Unit",
                    "Hole",
                    "Native",
                    "External",
                ],
            )),
        }
    }

    /// Classic tagged array format: ["Literal", attrs, literal]
    fn visit_seq<V>(self, mut seq: V) -> Result<Value<TA, VA>, V::Error>
    where
        V: SeqAccess<'de>,
    {
        let tag: String = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;

        match tag.as_str() {
            "Literal" | "literal" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let lit: Literal = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Value::Literal(attrs, lit))
            }
            "Constructor" | "constructor" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let name: FQName = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Value::Constructor(attrs, name))
            }
            "Tuple" | "tuple" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let elements: Vec<Value<TA, VA>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Value::Tuple(attrs, elements))
            }
            "List" | "list" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let elements: Vec<Value<TA, VA>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Value::List(attrs, elements))
            }
            "Record" | "record" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let fields: Vec<RecordFieldEntry<TA, VA>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Value::Record(attrs, fields))
            }
            "Variable" | "variable" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let name: Name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Value::Variable(attrs, name))
            }
            "Reference" | "reference" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let fqname: FQName = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Value::Reference(attrs, fqname))
            }
            "Field" | "field" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let record: Value<TA, VA> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let field_name: Name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Value::Field(attrs, Box::new(record), field_name))
            }
            "FieldFunction" | "fieldFunction" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let name: Name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                Ok(Value::FieldFunction(attrs, name))
            }
            "Apply" | "apply" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let func: Value<TA, VA> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let arg: Value<TA, VA> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Value::Apply(attrs, Box::new(func), Box::new(arg)))
            }
            "Lambda" | "lambda" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let pattern: Pattern<VA> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let body: Value<TA, VA> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Value::Lambda(attrs, pattern, Box::new(body)))
            }
            "LetDefinition" | "letDefinition" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let name: Name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let def: ValueDefinition<TA, VA> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                let body: Value<TA, VA> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(4, &self))?;
                Ok(Value::LetDefinition(
                    attrs,
                    name,
                    Box::new(def),
                    Box::new(body),
                ))
            }
            "LetRecursion" | "letRecursion" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let defs: Vec<LetBinding<TA, VA>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let body: Value<TA, VA> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Value::LetRecursion(attrs, defs, Box::new(body)))
            }
            "Destructure" | "destructure" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let pattern: Pattern<VA> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let val: Value<TA, VA> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                let body: Value<TA, VA> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(4, &self))?;
                Ok(Value::Destructure(
                    attrs,
                    pattern,
                    Box::new(val),
                    Box::new(body),
                ))
            }
            "IfThenElse" | "ifThenElse" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let cond: Value<TA, VA> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let then_branch: Value<TA, VA> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                let else_branch: Value<TA, VA> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(4, &self))?;
                Ok(Value::IfThenElse(
                    attrs,
                    Box::new(cond),
                    Box::new(then_branch),
                    Box::new(else_branch),
                ))
            }
            "PatternMatch" | "patternMatch" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let input: Value<TA, VA> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let cases: Vec<PatternCase<TA, VA>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Value::PatternMatch(attrs, Box::new(input), cases))
            }
            "UpdateRecord" | "updateRecord" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let record: Value<TA, VA> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let updates: Vec<RecordFieldEntry<TA, VA>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Value::UpdateRecord(attrs, Box::new(record), updates))
            }
            "Unit" | "unit" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Value::Unit(attrs))
            }
            // V4-only variants
            "Hole" | "hole" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let reason: HoleReason = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let expected_type: Option<Box<Type<TA>>> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Value::Hole(attrs, reason, expected_type))
            }
            "Native" | "native" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let fqname: FQName = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let info: NativeInfo = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Value::Native(attrs, fqname, info))
            }
            "External" | "external" => {
                let attrs: VA = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let external_name: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let target_platform: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                Ok(Value::External(attrs, external_name, target_platform))
            }
            _ => Err(de::Error::unknown_variant(
                &tag,
                &[
                    "Literal",
                    "Constructor",
                    "Tuple",
                    "List",
                    "Record",
                    "Variable",
                    "Reference",
                    "Field",
                    "FieldFunction",
                    "Apply",
                    "Lambda",
                    "LetDefinition",
                    "LetRecursion",
                    "Destructure",
                    "IfThenElse",
                    "PatternMatch",
                    "UpdateRecord",
                    "Unit",
                    "Hole",
                    "Native",
                    "External",
                ],
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_variable_roundtrip() {
        let var: Type<serde_json::Value> = Type::Variable(
            serde_json::Value::Object(Default::default()),
            Name::from("a"),
        );
        let json = serde_json::to_string(&var).unwrap();
        assert!(json.contains("Variable"));

        let parsed: Type<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, Type::Variable(_, _)));
    }

    #[test]
    fn test_type_unit_roundtrip() {
        let unit: Type<serde_json::Value> =
            Type::Unit(serde_json::Value::Object(Default::default()));
        let json = serde_json::to_string(&unit).unwrap();
        assert!(json.contains("Unit"));

        let parsed: Type<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, Type::Unit(_)));
    }

    #[test]
    fn test_type_function_roundtrip() {
        let func: Type<serde_json::Value> = Type::Function(
            serde_json::Value::Object(Default::default()),
            Box::new(Type::Unit(serde_json::Value::Object(Default::default()))),
            Box::new(Type::Unit(serde_json::Value::Object(Default::default()))),
        );
        let json = serde_json::to_string(&func).unwrap();
        assert!(json.contains("Function"));

        let parsed: Type<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, Type::Function(_, _, _)));
    }

    #[test]
    fn test_pattern_wildcard_roundtrip() {
        let pattern: Pattern<serde_json::Value> =
            Pattern::WildcardPattern(serde_json::Value::Object(Default::default()));
        let json = serde_json::to_string(&pattern).unwrap();
        assert!(json.contains("WildcardPattern"));

        let parsed: Pattern<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, Pattern::WildcardPattern(_)));
    }

    #[test]
    fn test_pattern_literal_roundtrip() {
        let pattern: Pattern<serde_json::Value> = Pattern::LiteralPattern(
            serde_json::Value::Object(Default::default()),
            Literal::Integer(42),
        );
        let json = serde_json::to_string(&pattern).unwrap();
        assert!(json.contains("LiteralPattern"));

        let parsed: Pattern<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            parsed,
            Pattern::LiteralPattern(_, Literal::Integer(42))
        ));
    }

    #[test]
    fn test_lowercase_tag_compatibility() {
        // Classic format sometimes uses lowercase tags
        let json = r#"["variable", {}, ["x"]]"#;
        let parsed: Type<serde_json::Value> = serde_json::from_str(json).unwrap();
        assert!(matches!(parsed, Type::Variable(_, _)));
    }

    #[test]
    fn test_value_literal_serialization() {
        let val: Value<serde_json::Value, serde_json::Value> = Value::Literal(
            serde_json::Value::Object(Default::default()),
            Literal::Integer(42),
        );
        let json = serde_json::to_string(&val).unwrap();
        assert!(json.contains("Literal"));
        assert!(json.contains("IntegerLiteral"));
    }

    #[test]
    fn test_value_unit_serialization() {
        let val: Value<serde_json::Value, serde_json::Value> =
            Value::Unit(serde_json::Value::Object(Default::default()));
        let json = serde_json::to_string(&val).unwrap();
        assert!(json.contains("Unit"));
    }

    #[test]
    fn test_value_variable_serialization() {
        let val: Value<serde_json::Value, serde_json::Value> = Value::Variable(
            serde_json::Value::Object(Default::default()),
            Name::from("x"),
        );
        let json = serde_json::to_string(&val).unwrap();
        assert!(json.contains("Variable"));
    }

    #[test]
    fn test_value_apply_serialization() {
        let val: Value<serde_json::Value, serde_json::Value> = Value::Apply(
            serde_json::Value::Object(Default::default()),
            Box::new(Value::Unit(serde_json::Value::Object(Default::default()))),
            Box::new(Value::Unit(serde_json::Value::Object(Default::default()))),
        );
        let json = serde_json::to_string(&val).unwrap();
        assert!(json.contains("Apply"));
    }

    #[test]
    fn test_hole_reason_serialization() {
        // Test UnresolvedReference variant
        let target = FQName::from_canonical_string("test:module#func").unwrap();
        let reason = HoleReason::UnresolvedReference { target };
        let json = serde_json::to_string(&reason).unwrap();
        assert!(json.contains("UnresolvedReference"));
        assert!(json.contains("target"));

        let parsed: HoleReason = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, HoleReason::UnresolvedReference { .. }));

        // Test TypeMismatch variant
        let reason2 = HoleReason::TypeMismatch {
            expected: "Int".to_string(),
            found: "String".to_string(),
        };
        let json2 = serde_json::to_string(&reason2).unwrap();
        assert!(json2.contains("TypeMismatch"));
        assert!(json2.contains("expected"));
        assert!(json2.contains("found"));

        let parsed2: HoleReason = serde_json::from_str(&json2).unwrap();
        assert!(matches!(parsed2, HoleReason::TypeMismatch { .. }));
    }

    #[test]
    fn test_native_hint_roundtrip() {
        let hint = NativeHint::Arithmetic;
        let json = serde_json::to_string(&hint).unwrap();
        assert_eq!(json, "\"Arithmetic\"");

        let parsed: NativeHint = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, NativeHint::Arithmetic));
    }

    #[test]
    fn test_native_info_roundtrip() {
        let info = NativeInfo {
            hint: NativeHint::StringOp,
            description: Some("String operation".to_string()),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("StringOp"));
        assert!(json.contains("String operation"));

        let parsed: NativeInfo = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed.hint, NativeHint::StringOp));
        assert_eq!(parsed.description, Some("String operation".to_string()));
    }

    // ==========================================================================
    // V4 Format Tests
    // ==========================================================================

    #[test]
    fn test_type_serializes_to_v4_wrapper_format() {
        // Type should now serialize to V4 wrapper object format
        let var: Type<()> = Type::Variable((), Name::from("a"));
        let json = serde_json::to_string(&var).unwrap();

        // V4 format: {"Variable": {"name": "a"}}
        assert!(json.contains(r#""Variable""#));
        assert!(json.contains(r#""name""#));
        assert!(json.contains(r#""a""#));

        // Should NOT be classic array format
        assert!(!json.starts_with('['));
    }

    #[test]
    fn test_type_deserializes_from_v4_format() {
        // V4 object wrapper format
        let json = r#"{"Variable": {"name": "a"}}"#;
        let parsed: Type<()> = serde_json::from_str(json).unwrap();

        match parsed {
            Type::Variable(_, name) => {
                assert_eq!(name.to_string(), "a");
            }
            _ => panic!("Expected Variable"),
        }
    }

    #[test]
    fn test_type_deserializes_from_classic_format() {
        // Classic array format should still work for backward compatibility
        let json = r#"["Variable", null, "a"]"#;
        let parsed: Type<()> = serde_json::from_str(json).unwrap();

        match parsed {
            Type::Variable(_, name) => {
                assert_eq!(name.to_string(), "a");
            }
            _ => panic!("Expected Variable"),
        }
    }

    #[test]
    fn test_type_deserializes_from_string_shorthand() {
        // V4 string shorthand for Variable
        let json = r#""x""#;
        let parsed: Type<()> = serde_json::from_str(json).unwrap();

        match parsed {
            Type::Variable(_, name) => {
                assert_eq!(name.to_string(), "x");
            }
            _ => panic!("Expected Variable"),
        }
    }

    #[test]
    fn test_type_reference_deserializes_from_fqname_shorthand() {
        // V4 string shorthand for Reference with FQName
        let json = r#""morphir/sdk:basics#int""#;
        let parsed: Type<()> = serde_json::from_str(json).unwrap();

        match parsed {
            Type::Reference(_, fqname, args) => {
                assert_eq!(fqname.to_canonical_string(), "morphir/sdk:basics#int");
                assert!(args.is_empty());
            }
            _ => panic!("Expected Reference"),
        }
    }

    #[test]
    fn test_type_reference_v4_roundtrip() {
        let tpe: Type<()> = Type::Reference(
            (),
            FQName::from_canonical_string("morphir/sdk:basics#int").unwrap(),
            vec![],
        );
        let json = serde_json::to_string(&tpe).unwrap();

        // V4 format: {"Reference": {"fqname": "morphir/sdk:basics#int"}}
        assert!(json.contains(r#""Reference""#));
        assert!(json.contains(r#""fqname""#));
        assert!(json.contains(r#"morphir/sdk:basics#int"#));

        let parsed: Type<()> = serde_json::from_str(&json).unwrap();
        match parsed {
            Type::Reference(_, fqname, args) => {
                assert_eq!(fqname.to_canonical_string(), "morphir/sdk:basics#int");
                assert!(args.is_empty());
            }
            _ => panic!("Expected Reference"),
        }
    }

    #[test]
    fn test_type_function_v4_roundtrip() {
        let tpe: Type<()> = Type::Function(
            (),
            Box::new(Type::Variable((), Name::from("a"))),
            Box::new(Type::Variable((), Name::from("b"))),
        );
        let json = serde_json::to_string(&tpe).unwrap();

        // V4 format: {"Function": {"arg": ..., "result": ...}}
        assert!(json.contains(r#""Function""#));
        assert!(json.contains(r#""arg""#));
        assert!(json.contains(r#""result""#));

        let parsed: Type<()> = serde_json::from_str(&json).unwrap();
        match parsed {
            Type::Function(_, arg, result) => {
                assert!(matches!(*arg, Type::Variable(_, _)));
                assert!(matches!(*result, Type::Variable(_, _)));
            }
            _ => panic!("Expected Function"),
        }
    }

    #[test]
    fn test_type_record_v4_format() {
        let tpe: Type<()> = Type::Record(
            (),
            vec![
                Field {
                    name: Name::from("name"),
                    tpe: Type::Variable((), Name::from("a")),
                },
                Field {
                    name: Name::from("age"),
                    tpe: Type::Variable((), Name::from("b")),
                },
            ],
        );
        let json = serde_json::to_string(&tpe).unwrap();

        // V4 format: {"Record": {"fields": {"name": ..., "age": ...}}}
        assert!(json.contains(r#""Record""#));
        assert!(json.contains(r#""fields""#));

        let parsed: Type<()> = serde_json::from_str(&json).unwrap();
        match parsed {
            Type::Record(_, fields) => {
                assert_eq!(fields.len(), 2);
            }
            _ => panic!("Expected Record"),
        }
    }

    // ==========================================================================
    // Pattern V4 Format Tests
    // ==========================================================================

    #[test]
    fn test_pattern_serializes_to_v4_wrapper_format() {
        let pattern: Pattern<()> = Pattern::WildcardPattern(());
        let json = serde_json::to_string(&pattern).unwrap();

        // V4 format: {"WildcardPattern": {}}
        assert!(json.contains(r#""WildcardPattern""#));
        assert!(!json.starts_with('['));
    }

    #[test]
    fn test_pattern_deserializes_from_v4_format() {
        let json = r#"{"WildcardPattern": {}}"#;
        let parsed: Pattern<()> = serde_json::from_str(json).unwrap();
        assert!(matches!(parsed, Pattern::WildcardPattern(_)));
    }

    #[test]
    fn test_pattern_deserializes_from_classic_format() {
        let json = r#"["WildcardPattern", null]"#;
        let parsed: Pattern<()> = serde_json::from_str(json).unwrap();
        assert!(matches!(parsed, Pattern::WildcardPattern(_)));
    }

    #[test]
    fn test_pattern_tuple_v4_roundtrip() {
        let pattern: Pattern<()> = Pattern::TuplePattern(
            (),
            vec![
                Pattern::WildcardPattern(()),
                Pattern::WildcardPattern(()),
            ],
        );
        let json = serde_json::to_string(&pattern).unwrap();

        assert!(json.contains(r#""TuplePattern""#));
        assert!(json.contains(r#""elements""#));

        let parsed: Pattern<()> = serde_json::from_str(&json).unwrap();
        match parsed {
            Pattern::TuplePattern(_, elements) => {
                assert_eq!(elements.len(), 2);
            }
            _ => panic!("Expected TuplePattern"),
        }
    }

    #[test]
    fn test_pattern_literal_v4_roundtrip() {
        let pattern: Pattern<()> = Pattern::LiteralPattern((), Literal::Integer(42));
        let json = serde_json::to_string(&pattern).unwrap();

        assert!(json.contains(r#""LiteralPattern""#));
        assert!(json.contains(r#""IntegerLiteral""#));
        assert!(json.contains("42"));

        let parsed: Pattern<()> = serde_json::from_str(&json).unwrap();
        match parsed {
            Pattern::LiteralPattern(_, Literal::Integer(v)) => {
                assert_eq!(v, 42);
            }
            _ => panic!("Expected LiteralPattern with Integer"),
        }
    }

    // ==========================================================================
    // Literal V4 Format Tests
    // ==========================================================================

    #[test]
    fn test_literal_serializes_to_v4_wrapper_format() {
        let lit = Literal::Integer(42);
        let json = serde_json::to_string(&lit).unwrap();

        // V4 format: {"IntegerLiteral": {"value": 42}}
        assert!(json.contains(r#""IntegerLiteral""#));
        assert!(json.contains(r#""value""#));
        assert!(json.contains("42"));
        assert!(!json.starts_with('['));
    }

    #[test]
    fn test_literal_deserializes_from_v4_format() {
        let json = r#"{"IntegerLiteral": {"value": 42}}"#;
        let parsed: Literal = serde_json::from_str(json).unwrap();
        assert_eq!(parsed, Literal::Integer(42));
    }

    #[test]
    fn test_literal_deserializes_from_classic_format() {
        let json = r#"["IntegerLiteral", 42]"#;
        let parsed: Literal = serde_json::from_str(json).unwrap();
        assert_eq!(parsed, Literal::Integer(42));
    }

    #[test]
    fn test_literal_string_v4_roundtrip() {
        let lit = Literal::String("hello".to_string());
        let json = serde_json::to_string(&lit).unwrap();

        assert!(json.contains(r#""StringLiteral""#));
        assert!(json.contains("hello"));

        let parsed: Literal = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, Literal::String("hello".to_string()));
    }

    #[test]
    fn test_literal_bool_v4_roundtrip() {
        let lit = Literal::Bool(true);
        let json = serde_json::to_string(&lit).unwrap();

        assert!(json.contains(r#""BoolLiteral""#));

        let parsed: Literal = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, Literal::Bool(true));
    }

    // ==========================================================================
    // Value V4 Format Tests
    // ==========================================================================

    #[test]
    fn test_value_serializes_to_v4_wrapper_format() {
        let val: Value<(), ()> = Value::Unit(());
        let json = serde_json::to_string(&val).unwrap();

        // V4 format: {"Unit": {}}
        assert!(json.contains(r#""Unit""#));
        assert!(!json.starts_with('['));
    }

    #[test]
    fn test_value_deserializes_from_v4_format() {
        let json = r#"{"Unit": {}}"#;
        let parsed: Value<(), ()> = serde_json::from_str(json).unwrap();
        assert!(matches!(parsed, Value::Unit(_)));
    }

    #[test]
    fn test_value_deserializes_from_classic_format() {
        let json = r#"["Unit", null]"#;
        let parsed: Value<(), ()> = serde_json::from_str(json).unwrap();
        assert!(matches!(parsed, Value::Unit(_)));
    }

    #[test]
    fn test_value_variable_v4_roundtrip() {
        let val: Value<(), ()> = Value::Variable((), Name::from("x"));
        let json = serde_json::to_string(&val).unwrap();

        assert!(json.contains(r#""Variable""#));
        assert!(json.contains(r#""name""#));

        let parsed: Value<(), ()> = serde_json::from_str(&json).unwrap();
        match parsed {
            Value::Variable(_, name) => {
                assert_eq!(name.to_string(), "x");
            }
            _ => panic!("Expected Variable"),
        }
    }

    #[test]
    fn test_value_literal_v4_roundtrip() {
        let val: Value<(), ()> = Value::Literal((), Literal::Integer(123));
        let json = serde_json::to_string(&val).unwrap();

        assert!(json.contains(r#""Literal""#));
        assert!(json.contains(r#""IntegerLiteral""#));
        assert!(json.contains("123"));

        let parsed: Value<(), ()> = serde_json::from_str(&json).unwrap();
        match parsed {
            Value::Literal(_, Literal::Integer(v)) => {
                assert_eq!(v, 123);
            }
            _ => panic!("Expected Literal with Integer"),
        }
    }

    #[test]
    fn test_value_tuple_v4_roundtrip() {
        let val: Value<(), ()> = Value::Tuple(
            (),
            vec![Value::Unit(()), Value::Unit(())],
        );
        let json = serde_json::to_string(&val).unwrap();

        assert!(json.contains(r#""Tuple""#));
        assert!(json.contains(r#""elements""#));

        let parsed: Value<(), ()> = serde_json::from_str(&json).unwrap();
        match parsed {
            Value::Tuple(_, elements) => {
                assert_eq!(elements.len(), 2);
            }
            _ => panic!("Expected Tuple"),
        }
    }

    #[test]
    fn test_value_apply_v4_roundtrip() {
        let val: Value<(), ()> = Value::Apply(
            (),
            Box::new(Value::Variable((), Name::from("f"))),
            Box::new(Value::Variable((), Name::from("x"))),
        );
        let json = serde_json::to_string(&val).unwrap();

        assert!(json.contains(r#""Apply""#));
        assert!(json.contains(r#""function""#));
        assert!(json.contains(r#""argument""#));

        let parsed: Value<(), ()> = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, Value::Apply(_, _, _)));
    }

    #[test]
    fn test_value_lambda_v4_roundtrip() {
        let val: Value<(), ()> = Value::Lambda(
            (),
            Pattern::WildcardPattern(()),
            Box::new(Value::Unit(())),
        );
        let json = serde_json::to_string(&val).unwrap();

        assert!(json.contains(r#""Lambda""#));
        assert!(json.contains(r#""argumentPattern""#));
        assert!(json.contains(r#""body""#));

        let parsed: Value<(), ()> = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, Value::Lambda(_, _, _)));
    }

    #[test]
    fn test_value_if_then_else_v4_roundtrip() {
        let val: Value<(), ()> = Value::IfThenElse(
            (),
            Box::new(Value::Literal((), Literal::Bool(true))),
            Box::new(Value::Unit(())),
            Box::new(Value::Unit(())),
        );
        let json = serde_json::to_string(&val).unwrap();

        assert!(json.contains(r#""IfThenElse""#));
        assert!(json.contains(r#""condition""#));
        assert!(json.contains(r#""thenBranch""#));
        assert!(json.contains(r#""elseBranch""#));

        let parsed: Value<(), ()> = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, Value::IfThenElse(_, _, _, _)));
    }

    #[test]
    fn test_value_reference_v4_roundtrip() {
        let val: Value<(), ()> = Value::Reference(
            (),
            FQName::from_canonical_string("morphir/sdk:basics#add").unwrap(),
        );
        let json = serde_json::to_string(&val).unwrap();

        assert!(json.contains(r#""Reference""#));
        assert!(json.contains(r#""fqname""#));
        assert!(json.contains("morphir/sdk:basics#add"));

        let parsed: Value<(), ()> = serde_json::from_str(&json).unwrap();
        match parsed {
            Value::Reference(_, fqname) => {
                assert_eq!(fqname.to_canonical_string(), "morphir/sdk:basics#add");
            }
            _ => panic!("Expected Reference"),
        }
    }

    #[test]
    fn test_value_list_v4_roundtrip() {
        let val: Value<(), ()> = Value::List(
            (),
            vec![
                Value::Literal((), Literal::Integer(1)),
                Value::Literal((), Literal::Integer(2)),
            ],
        );
        let json = serde_json::to_string(&val).unwrap();

        assert!(json.contains(r#""List""#));
        assert!(json.contains(r#""items""#));

        let parsed: Value<(), ()> = serde_json::from_str(&json).unwrap();
        match parsed {
            Value::List(_, elements) => {
                assert_eq!(elements.len(), 2);
            }
            _ => panic!("Expected List"),
        }
    }

    #[test]
    fn test_value_field_v4_roundtrip() {
        let val: Value<(), ()> = Value::Field(
            (),
            Box::new(Value::Variable((), Name::from("record"))),
            Name::from("myField"),
        );
        let json = serde_json::to_string(&val).unwrap();

        assert!(json.contains(r#""Field""#));
        assert!(json.contains(r#""value""#));
        assert!(json.contains(r#""name""#));

        let parsed: Value<(), ()> = serde_json::from_str(&json).unwrap();
        match parsed {
            Value::Field(_, _, name) => {
                // Name uses title case conversion
                assert_eq!(name.to_string(), "my-field");
            }
            _ => panic!("Expected Field"),
        }
    }

    #[test]
    fn test_value_constructor_v4_roundtrip() {
        let val: Value<(), ()> = Value::Constructor(
            (),
            FQName::from_canonical_string("my/pkg:mod#MyType").unwrap(),
        );
        let json = serde_json::to_string(&val).unwrap();

        assert!(json.contains(r#""Constructor""#));
        assert!(json.contains(r#""fqname""#));

        let parsed: Value<(), ()> = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, Value::Constructor(_, _)));
    }
}

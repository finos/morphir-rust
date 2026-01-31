//! Tagged array serialization for Morphir IR.
//!
//! Morphir IR uses a tagged array format for serialization:
//! - `["Variable", attrs, name]`
//! - `["Reference", attrs, fqname, params]`
//! - `["Tuple", attrs, elements]`
//! - etc.
//!
//! This module provides Serialize/Deserialize implementations for Type, Pattern, and Value
//! using this format.

use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::ser::{SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::marker::PhantomData;

use super::literal::Literal;
use super::pattern::Pattern;
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
        match self {
            Type::Variable(attrs, name) => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element("Variable")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(name)?;
                seq.end()
            }
            Type::Reference(attrs, fqname, params) => {
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element("Reference")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(fqname)?;
                seq.serialize_element(params)?;
                seq.end()
            }
            Type::Tuple(attrs, elements) => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element("Tuple")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(elements)?;
                seq.end()
            }
            Type::Record(attrs, fields) => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element("Record")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(fields)?;
                seq.end()
            }
            Type::ExtensibleRecord(attrs, var, fields) => {
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element("ExtensibleRecord")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(var)?;
                seq.serialize_element(fields)?;
                seq.end()
            }
            Type::Function(attrs, arg, result) => {
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element("Function")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(arg)?;
                seq.serialize_element(result)?;
                seq.end()
            }
            Type::Unit(attrs) => {
                let mut seq = serializer.serialize_seq(Some(2))?;
                seq.serialize_element("Unit")?;
                seq.serialize_element(attrs)?;
                seq.end()
            }
        }
    }
}

impl<'de, A: Clone + Deserialize<'de>> Deserialize<'de> for Type<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(TypeVisitor(PhantomData))
    }
}

struct TypeVisitor<A>(PhantomData<A>);

impl<'de, A: Clone + Deserialize<'de>> Visitor<'de> for TypeVisitor<A> {
    type Value = Type<A>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a tagged array like [\"Variable\", attrs, name]")
    }

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

impl<'de, A: Clone + Deserialize<'de>> Deserialize<'de> for Field<A> {
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

        impl<'de, A: Clone + Deserialize<'de>> Visitor<'de> for FieldVisitor<A> {
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
        match self {
            Pattern::WildcardPattern(attrs) => {
                let mut seq = serializer.serialize_seq(Some(2))?;
                seq.serialize_element("WildcardPattern")?;
                seq.serialize_element(attrs)?;
                seq.end()
            }
            Pattern::AsPattern(attrs, pattern, name) => {
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element("AsPattern")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(pattern)?;
                seq.serialize_element(name)?;
                seq.end()
            }
            Pattern::TuplePattern(attrs, elements) => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element("TuplePattern")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(elements)?;
                seq.end()
            }
            Pattern::ConstructorPattern(attrs, name, args) => {
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element("ConstructorPattern")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(name)?;
                seq.serialize_element(args)?;
                seq.end()
            }
            Pattern::EmptyListPattern(attrs) => {
                let mut seq = serializer.serialize_seq(Some(2))?;
                seq.serialize_element("EmptyListPattern")?;
                seq.serialize_element(attrs)?;
                seq.end()
            }
            Pattern::HeadTailPattern(attrs, head, tail) => {
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element("HeadTailPattern")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(head)?;
                seq.serialize_element(tail)?;
                seq.end()
            }
            Pattern::LiteralPattern(attrs, lit) => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element("LiteralPattern")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(lit)?;
                seq.end()
            }
            Pattern::UnitPattern(attrs) => {
                let mut seq = serializer.serialize_seq(Some(2))?;
                seq.serialize_element("UnitPattern")?;
                seq.serialize_element(attrs)?;
                seq.end()
            }
        }
    }
}

impl<'de, A: Clone + Deserialize<'de>> Deserialize<'de> for Pattern<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(PatternVisitor(PhantomData))
    }
}

struct PatternVisitor<A>(PhantomData<A>);

impl<'de, A: Clone + Deserialize<'de>> Visitor<'de> for PatternVisitor<A> {
    type Value = Pattern<A>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a tagged array like [\"WildcardPattern\", attrs]")
    }

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
        match self {
            Value::Literal(attrs, lit) => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element("Literal")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(lit)?;
                seq.end()
            }
            Value::Constructor(attrs, name) => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element("Constructor")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(name)?;
                seq.end()
            }
            Value::Tuple(attrs, elements) => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element("Tuple")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(elements)?;
                seq.end()
            }
            Value::List(attrs, elements) => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element("List")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(elements)?;
                seq.end()
            }
            Value::Record(attrs, fields) => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element("Record")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(fields)?;
                seq.end()
            }
            Value::Variable(attrs, name) => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element("Variable")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(name)?;
                seq.end()
            }
            Value::Reference(attrs, fqname) => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element("Reference")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(fqname)?;
                seq.end()
            }
            Value::Field(attrs, record, field_name) => {
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element("Field")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(record)?;
                seq.serialize_element(field_name)?;
                seq.end()
            }
            Value::FieldFunction(attrs, name) => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element("FieldFunction")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(name)?;
                seq.end()
            }
            Value::Apply(attrs, func, arg) => {
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element("Apply")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(func)?;
                seq.serialize_element(arg)?;
                seq.end()
            }
            Value::Lambda(attrs, pattern, body) => {
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element("Lambda")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(pattern)?;
                seq.serialize_element(body)?;
                seq.end()
            }
            Value::LetDefinition(attrs, name, def, body) => {
                let mut seq = serializer.serialize_seq(Some(5))?;
                seq.serialize_element("LetDefinition")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(name)?;
                seq.serialize_element(def)?;
                seq.serialize_element(body)?;
                seq.end()
            }
            Value::LetRecursion(attrs, defs, body) => {
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element("LetRecursion")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(defs)?;
                seq.serialize_element(body)?;
                seq.end()
            }
            Value::Destructure(attrs, pattern, val, body) => {
                let mut seq = serializer.serialize_seq(Some(5))?;
                seq.serialize_element("Destructure")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(pattern)?;
                seq.serialize_element(val)?;
                seq.serialize_element(body)?;
                seq.end()
            }
            Value::IfThenElse(attrs, cond, then_branch, else_branch) => {
                let mut seq = serializer.serialize_seq(Some(5))?;
                seq.serialize_element("IfThenElse")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(cond)?;
                seq.serialize_element(then_branch)?;
                seq.serialize_element(else_branch)?;
                seq.end()
            }
            Value::PatternMatch(attrs, input, cases) => {
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element("PatternMatch")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(input)?;
                seq.serialize_element(cases)?;
                seq.end()
            }
            Value::UpdateRecord(attrs, record, updates) => {
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element("UpdateRecord")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(record)?;
                seq.serialize_element(updates)?;
                seq.end()
            }
            Value::Unit(attrs) => {
                let mut seq = serializer.serialize_seq(Some(2))?;
                seq.serialize_element("Unit")?;
                seq.serialize_element(attrs)?;
                seq.end()
            }
            // V4-only variants
            Value::Hole(attrs, reason, expected_type) => {
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element("Hole")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(reason)?;
                seq.serialize_element(expected_type)?;
                seq.end()
            }
            Value::Native(attrs, fqname, info) => {
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element("Native")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(fqname)?;
                seq.serialize_element(info)?;
                seq.end()
            }
            Value::External(attrs, external_name, target_platform) => {
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element("External")?;
                seq.serialize_element(attrs)?;
                seq.serialize_element(external_name)?;
                seq.serialize_element(target_platform)?;
                seq.end()
            }
        }
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

impl<'de, TA: Clone + Deserialize<'de>, VA: Clone + Deserialize<'de>> Deserialize<'de>
    for InputType<TA, VA>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct InputTypeVisitor<TA, VA>(PhantomData<(TA, VA)>);

        impl<'de, TA: Clone + Deserialize<'de>, VA: Clone + Deserialize<'de>> Visitor<'de>
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

impl<'de, TA: Clone + Deserialize<'de>, VA: Clone + Deserialize<'de>> Deserialize<'de>
    for RecordFieldEntry<TA, VA>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RecordFieldEntryVisitor<TA, VA>(PhantomData<(TA, VA)>);

        impl<'de, TA: Clone + Deserialize<'de>, VA: Clone + Deserialize<'de>> Visitor<'de>
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

impl<'de, TA: Clone + Deserialize<'de>, VA: Clone + Deserialize<'de>> Deserialize<'de>
    for PatternCase<TA, VA>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PatternCaseVisitor<TA, VA>(PhantomData<(TA, VA)>);

        impl<'de, TA: Clone + Deserialize<'de>, VA: Clone + Deserialize<'de>> Visitor<'de>
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

impl<'de, TA: Clone + Deserialize<'de>, VA: Clone + Deserialize<'de>> Deserialize<'de>
    for LetBinding<TA, VA>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LetBindingVisitor<TA, VA>(PhantomData<(TA, VA)>);

        impl<'de, TA: Clone + Deserialize<'de>, VA: Clone + Deserialize<'de>> Visitor<'de>
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

impl<'de, A: Clone + Deserialize<'de>> Deserialize<'de> for ConstructorArg<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ConstructorArgVisitor<A>(PhantomData<A>);

        impl<'de, A: Clone + Deserialize<'de>> Visitor<'de> for ConstructorArgVisitor<A> {
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
impl<'de, TA: Clone + Deserialize<'de>, VA: Clone + Deserialize<'de>> Deserialize<'de>
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

        impl<'de, TA: Clone + Deserialize<'de>, VA: Clone + Deserialize<'de>> Visitor<'de>
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
impl<'de, TA: Clone + Deserialize<'de>, VA: Clone + Deserialize<'de>> Deserialize<'de>
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

        impl<'de, TA: Clone + Deserialize<'de>, VA: Clone + Deserialize<'de>> Visitor<'de>
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
impl<'de, TA: Clone + Deserialize<'de>, VA: Clone + Deserialize<'de>> Deserialize<'de>
    for Value<TA, VA>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(ValueVisitor(PhantomData))
    }
}

struct ValueVisitor<TA, VA>(PhantomData<(TA, VA)>);

impl<'de, TA: Clone + Deserialize<'de>, VA: Clone + Deserialize<'de>> Visitor<'de>
    for ValueVisitor<TA, VA>
{
    type Value = Value<TA, VA>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a tagged array like [\"Literal\", attrs, literal]")
    }

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
}

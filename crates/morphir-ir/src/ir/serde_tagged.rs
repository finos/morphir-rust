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

use crate::naming::{FQName, Name};
use super::literal::Literal;
use super::pattern::Pattern;
use super::type_expr::{Field, Type};

// =============================================================================
// Type<A> Serialization
// =============================================================================

impl<A: Serialize> Serialize for Type<A> {
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

impl<'de, A: Deserialize<'de>> Deserialize<'de> for Type<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(TypeVisitor(PhantomData))
    }
}

struct TypeVisitor<A>(PhantomData<A>);

impl<'de, A: Deserialize<'de>> Visitor<'de> for TypeVisitor<A> {
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

impl<A: Serialize> Serialize for Field<A> {
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

impl<'de, A: Deserialize<'de>> Deserialize<'de> for Field<A> {
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

        impl<'de, A: Deserialize<'de>> Visitor<'de> for FieldVisitor<A> {
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

impl<A: Serialize> Serialize for Pattern<A> {
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

impl<'de, A: Deserialize<'de>> Deserialize<'de> for Pattern<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(PatternVisitor(PhantomData))
    }
}

struct PatternVisitor<A>(PhantomData<A>);

impl<'de, A: Deserialize<'de>> Visitor<'de> for PatternVisitor<A> {
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

use super::value_expr::{Value, ValueDefinition, ValueBody, HoleReason, NativeInfo, NativeHint};

impl<TA: Serialize, VA: Serialize> Serialize for Value<TA, VA> {
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
            HoleReason::DeletedDuringRefactor => serializer.serialize_str("DeletedDuringRefactor"),
            HoleReason::TypeMismatch => serializer.serialize_str("TypeMismatch"),
            HoleReason::Draft => serializer.serialize_str("Draft"),
        }
    }
}

impl<'de> Deserialize<'de> for HoleReason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Accept either a string or an object
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(s) => match s.as_str() {
                "DeletedDuringRefactor" => Ok(HoleReason::DeletedDuringRefactor),
                "TypeMismatch" => Ok(HoleReason::TypeMismatch),
                "Draft" => Ok(HoleReason::Draft),
                _ => Err(de::Error::unknown_variant(
                    &s,
                    &["DeletedDuringRefactor", "TypeMismatch", "Draft"],
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
                    _ => Err(de::Error::unknown_variant(kind, &["UnresolvedReference"])),
                }
            }
            _ => Err(de::Error::custom("expected string or object for HoleReason")),
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
            NativeHint::PlatformSpecific => serializer.serialize_str("PlatformSpecific"),
        }
    }
}

impl<'de> Deserialize<'de> for NativeHint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "Arithmetic" => Ok(NativeHint::Arithmetic),
            "Comparison" => Ok(NativeHint::Comparison),
            "StringOp" => Ok(NativeHint::StringOp),
            "CollectionOp" => Ok(NativeHint::CollectionOp),
            "PlatformSpecific" => Ok(NativeHint::PlatformSpecific),
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
impl<TA: Serialize, VA: Serialize> Serialize for ValueDefinition<TA, VA> {
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
impl<TA: Serialize, VA: Serialize> Serialize for ValueBody<TA, VA> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_variable_roundtrip() {
        let var: Type<serde_json::Value> =
            Type::Variable(serde_json::Value::Object(Default::default()), Name::from("a"));
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
        let reason = HoleReason::Draft;
        let json = serde_json::to_string(&reason).unwrap();
        assert_eq!(json, "\"Draft\"");

        let parsed: HoleReason = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, HoleReason::Draft));
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

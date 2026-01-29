//! Morphir Classic IR (Elm-compatible V1-V3)
//!
//! This module implements the Morphir IR structures exactly as they appear in the
//! `morphir-elm` reference implementation. It uses recursive types with generic
//! attributes and supports the specific array-based JSON serialization format used
//! by the classic Morphir tools.

use crate::naming::{FQName, Name, Path};
use indexmap::IndexMap;
use serde::de::{self, SeqAccess, Visitor};
use serde::ser::{SerializeTuple, Serializer};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

// ----------------------------------------------------------------------------
// Type System
// ----------------------------------------------------------------------------

/// Type with generic attributes
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type<A> {
    ExtensibleRecord(A, Name, Vec<Field<A>>),
    Function(A, Box<Type<A>>, Box<Type<A>>),
    Record(A, Vec<Field<A>>),
    Reference(A, FQName, Vec<Type<A>>),
    Tuple(A, Vec<Type<A>>),
    Unit(A),
    Variable(A, Name),
}

impl<A: Serialize> Serialize for Type<A> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Type::ExtensibleRecord(a, name, fields) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("extensible_record")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(name)?;
                tuple.serialize_element(fields)?;
                tuple.end()
            }
            Type::Function(a, arg, ret) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("function")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(arg)?;
                tuple.serialize_element(ret)?;
                tuple.end()
            }
            Type::Record(a, fields) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("record")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(fields)?;
                tuple.end()
            }
            Type::Reference(a, name, args) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("reference")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(name)?;
                tuple.serialize_element(args)?;
                tuple.end()
            }
            Type::Tuple(a, elements) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("tuple")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(elements)?;
                tuple.end()
            }
            Type::Unit(a) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("unit")?;
                tuple.serialize_element(a)?;
                tuple.end()
            }
            Type::Variable(a, name) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("variable")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(name)?;
                tuple.end()
            }
        }
    }
}

impl<'de, A: Deserialize<'de>> Deserialize<'de> for Type<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct TypeVisitor<A>(std::marker::PhantomData<A>);

        impl<'de, A: Deserialize<'de>> Visitor<'de> for TypeVisitor<A> {
            type Value = Type<A>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a classic Type array")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let tag: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;

                match tag.as_str() {
                    "extensible_record" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let fields = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        Ok(Type::ExtensibleRecord(a, name, fields))
                    }
                    "function" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let arg = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let ret = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        Ok(Type::Function(a, arg, ret))
                    }
                    "record" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let fields = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(Type::Record(a, fields))
                    }
                    "reference" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let args = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        Ok(Type::Reference(a, name, args))
                    }
                    "tuple" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let elements = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(Type::Tuple(a, elements))
                    }
                    "unit" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        Ok(Type::Unit(a))
                    }
                    "variable" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(Type::Variable(a, name))
                    }
                    _ => Err(de::Error::unknown_variant(
                        &tag,
                        &[
                            "extensible_record",
                            "function",
                            "record",
                            "reference",
                            "tuple",
                            "unit",
                            "variable",
                        ],
                    )),
                }
            }
        }

        deserializer.deserialize_seq(TypeVisitor(std::marker::PhantomData))
    }
}

/// Record field definition - serialized as ["name", type]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field<A> {
    pub name: Name,
    pub tpe: Type<A>,
}

impl<A: Serialize> Serialize for Field<A> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut tuple = serializer.serialize_tuple(2)?;
        tuple.serialize_element(&self.name)?;
        tuple.serialize_element(&self.tpe)?;
        tuple.end()
    }
}

impl<'de, A: Deserialize<'de>> Deserialize<'de> for Field<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FieldVisitor<A>(std::marker::PhantomData<A>);
        impl<'de, A: Deserialize<'de>> Visitor<'de> for FieldVisitor<A> {
            type Value = Field<A>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a field array [name, type]")
            }
            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let tpe = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Field { name, tpe })
            }
        }
        deserializer.deserialize_seq(FieldVisitor(std::marker::PhantomData))
    }
}

/// Type specification (opaque, alias, or custom)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeSpecification<A> {
    TypeAliasSpecification(Vec<Name>, Type<A>),
    OpaqueTypeSpecification(Vec<Name>),
    CustomTypeSpecification(Vec<Name>, Vec<Constructor<A>>),
}

impl<A: Serialize> Serialize for TypeSpecification<A> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            TypeSpecification::TypeAliasSpecification(params, tpe) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("type_alias_specification")?;
                tuple.serialize_element(params)?;
                tuple.serialize_element(tpe)?;
                tuple.end()
            }
            TypeSpecification::OpaqueTypeSpecification(params) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("opaque_type_specification")?;
                tuple.serialize_element(params)?;
                tuple.end()
            }
            TypeSpecification::CustomTypeSpecification(params, ctors) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("custom_type_specification")?;
                tuple.serialize_element(params)?;
                tuple.serialize_element(ctors)?;
                tuple.end()
            }
        }
    }
}

impl<'de, A: Deserialize<'de>> Deserialize<'de> for TypeSpecification<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct TSVisitor<A>(std::marker::PhantomData<A>);
        impl<'de, A: Deserialize<'de>> Visitor<'de> for TSVisitor<A> {
            type Value = TypeSpecification<A>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a TypeSpecification array")
            }
            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let tag: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                match tag.as_str() {
                    "type_alias_specification" => {
                        let params = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let tpe = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(TypeSpecification::TypeAliasSpecification(params, tpe))
                    }
                    "opaque_type_specification" => {
                        let params = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        Ok(TypeSpecification::OpaqueTypeSpecification(params))
                    }
                    "custom_type_specification" => {
                        let params = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let ctors = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(TypeSpecification::CustomTypeSpecification(params, ctors))
                    }
                    _ => Err(de::Error::unknown_variant(
                        &tag,
                        &[
                            "type_alias_specification",
                            "opaque_type_specification",
                            "custom_type_specification",
                        ],
                    )),
                }
            }
        }
        deserializer.deserialize_seq(TSVisitor(std::marker::PhantomData))
    }
}

/// Constructor definition - likely ["name", [args]]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constructor<A> {
    pub name: Name,
    pub args: Vec<(Name, Type<A>)>,
}

impl<A: Serialize> Serialize for Constructor<A> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut tuple = serializer.serialize_tuple(2)?;
        tuple.serialize_element(&self.name)?;
        tuple.serialize_element(&self.args)?;
        tuple.end()
    }
}

impl<'de, A: Deserialize<'de>> Deserialize<'de> for Constructor<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CVisitor<A>(std::marker::PhantomData<A>);
        impl<'de, A: Deserialize<'de>> Visitor<'de> for CVisitor<A> {
            type Value = Constructor<A>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a Constructor array [name, args]")
            }
            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let name = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let args = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Constructor { name, args })
            }
        }
        deserializer.deserialize_seq(CVisitor(std::marker::PhantomData))
    }
}

/// Access controlled content ["access", value]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessControlled<A> {
    pub access: Access,
    pub value: A,
}

impl<A: Serialize> Serialize for AccessControlled<A> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut tuple = serializer.serialize_tuple(2)?;
        tuple.serialize_element(&self.access)?;
        tuple.serialize_element(&self.value)?;
        tuple.end()
    }
}

impl<'de, A: Deserialize<'de>> Deserialize<'de> for AccessControlled<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ACVisitor<A>(std::marker::PhantomData<A>);
        impl<'de, A: Deserialize<'de>> Visitor<'de> for ACVisitor<A> {
            type Value = AccessControlled<A>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an AccessControlled array [access, value]")
            }
            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let access = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let value = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(AccessControlled { access, value })
            }
        }
        deserializer.deserialize_seq(ACVisitor(std::marker::PhantomData))
    }
}

/// Access level
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    Public,
    Private,
}

/// Type definition (alias or custom)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeDefinition<A> {
    TypeAliasDefinition(Vec<Name>, Type<A>),
    CustomTypeDefinition(Vec<Name>, AccessControlled<Vec<Constructor<A>>>),
}

impl<A: Serialize> Serialize for TypeDefinition<A> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            TypeDefinition::TypeAliasDefinition(params, tpe) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("type_alias_definition")?;
                tuple.serialize_element(params)?;
                tuple.serialize_element(tpe)?;
                tuple.end()
            }
            TypeDefinition::CustomTypeDefinition(params, ctors) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("custom_type_definition")?;
                tuple.serialize_element(params)?;
                tuple.serialize_element(ctors)?;
                tuple.end()
            }
        }
    }
}

impl<'de, A: Deserialize<'de>> Deserialize<'de> for TypeDefinition<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct TDVisitor<A>(std::marker::PhantomData<A>);
        impl<'de, A: Deserialize<'de>> Visitor<'de> for TDVisitor<A> {
            type Value = TypeDefinition<A>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a TypeDefinition array")
            }
            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let tag: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                match tag.as_str() {
                    "type_alias_definition" => {
                        let params = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let tpe = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(TypeDefinition::TypeAliasDefinition(params, tpe))
                    }
                    "custom_type_definition" => {
                        let params = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let ctors = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(TypeDefinition::CustomTypeDefinition(params, ctors))
                    }
                    _ => Err(de::Error::unknown_variant(
                        &tag,
                        &["type_alias_definition", "custom_type_definition"],
                    )),
                }
            }
        }
        deserializer.deserialize_seq(TDVisitor(std::marker::PhantomData))
    }
}

// ----------------------------------------------------------------------------
// Value System
// ----------------------------------------------------------------------------

// Value, Pattern, etc. to be implemented in next step if file too large...
// I will just put placeholders or minimal impls for now and use another call for Value to keep it safe.
// Wait, I should not leave it broken. I will implement Value as well.

// ... Value impl is skipped in this turn for brevity/safety, will append in next tool call ...
// Actually, I'll write the file with Type system serialization fully done, and define Value enum without impls (derive) for now, then fix it.
// Oh wait, I need to match the previous tool call where I planned to define Value.
// I will just define Value with standard derive for now and then update it in the next step to not lose focus.
// Or better, I'll continue writing the file.

/// Value with generic type and value attributes
#[derive(Debug, Clone, PartialEq)]
pub enum Value<TA, VA> {
    Apply(VA, Box<Value<TA, VA>>, Box<Value<TA, VA>>),
    Constructor(VA, FQName),
    Destructure(VA, Pattern<VA>, Box<Value<TA, VA>>, Box<Value<TA, VA>>),
    Field(VA, Box<Value<TA, VA>>, Name),
    FieldFunction(VA, Name),
    IfThenElse(
        VA,
        Box<Value<TA, VA>>,
        Box<Value<TA, VA>>,
        Box<Value<TA, VA>>,
    ),
    Lambda(VA, Pattern<VA>, Box<Value<TA, VA>>),
    LetDefinition(VA, Name, Definition<TA, VA>, Box<Value<TA, VA>>),
    LetRecursion(VA, Vec<(Name, Definition<TA, VA>)>, Box<Value<TA, VA>>),
    List(VA, Vec<Value<TA, VA>>),
    Literal(VA, Literal),
    PatternMatch(VA, Box<Value<TA, VA>>, Vec<(Pattern<VA>, Value<TA, VA>)>),
    Record(VA, Vec<(Name, Value<TA, VA>)>),
    Tuple(VA, Vec<Value<TA, VA>>),
    Unit(VA),
    Update(VA, Box<Value<TA, VA>>, Vec<(Name, Value<TA, VA>)>),
    Variable(VA, Name),
}

impl<TA: Serialize, VA: Serialize> Serialize for Value<TA, VA> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Value::Apply(va, func, arg) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("apply")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(func)?;
                tuple.serialize_element(arg)?;
                tuple.end()
            }
            Value::Constructor(va, name) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("constructor")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(name)?;
                tuple.end()
            }
            Value::Destructure(va, pattern, value, in_expr) => {
                let mut tuple = serializer.serialize_tuple(5)?;
                tuple.serialize_element("destructure")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(pattern)?;
                tuple.serialize_element(value)?;
                tuple.serialize_element(in_expr)?;
                tuple.end()
            }
            Value::Field(va, record, name) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("field")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(record)?;
                tuple.serialize_element(name)?;
                tuple.end()
            }
            Value::FieldFunction(va, name) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("field_function")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(name)?;
                tuple.end()
            }
            Value::IfThenElse(va, cond, then_branch, else_branch) => {
                let mut tuple = serializer.serialize_tuple(5)?;
                tuple.serialize_element("if_then_else")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(cond)?;
                tuple.serialize_element(then_branch)?;
                tuple.serialize_element(else_branch)?;
                tuple.end()
            }
            Value::Lambda(va, pattern, body) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("lambda")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(pattern)?;
                tuple.serialize_element(body)?;
                tuple.end()
            }
            Value::LetDefinition(va, name, def, in_expr) => {
                let mut tuple = serializer.serialize_tuple(5)?;
                tuple.serialize_element("let_definition")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(name)?;
                tuple.serialize_element(def)?;
                tuple.serialize_element(in_expr)?;
                tuple.end()
            }
            Value::LetRecursion(va, defs, in_expr) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("let_recursion")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(defs)?;
                tuple.serialize_element(in_expr)?;
                tuple.end()
            }
            Value::List(va, elements) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("list")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(elements)?;
                tuple.end()
            }
            Value::Literal(va, lit) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("literal")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(lit)?;
                tuple.end()
            }
            Value::PatternMatch(va, expr, cases) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("pattern_match")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(expr)?;
                tuple.serialize_element(cases)?;
                tuple.end()
            }
            Value::Record(va, fields) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("record")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(fields)?;
                tuple.end()
            }
            Value::Tuple(va, elements) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("tuple")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(elements)?;
                tuple.end()
            }
            Value::Unit(va) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("unit")?;
                tuple.serialize_element(va)?;
                tuple.end()
            }
            Value::Update(va, record, fields) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("update")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(record)?;
                tuple.serialize_element(fields)?;
                tuple.end()
            }
            Value::Variable(va, name) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("variable")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(name)?;
                tuple.end()
            }
        }
    }
}

impl<'de, TA: Deserialize<'de>, VA: Deserialize<'de>> Deserialize<'de> for Value<TA, VA> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ValueVisitor<TA, VA>(std::marker::PhantomData<(TA, VA)>);

        impl<'de, TA: Deserialize<'de>, VA: Deserialize<'de>> Visitor<'de> for ValueVisitor<TA, VA> {
            type Value = Value<TA, VA>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a classic Value array")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let tag: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;

                match tag.as_str() {
                    "apply" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let func = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let arg = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        Ok(Value::Apply(va, func, arg))
                    }
                    "constructor" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(Value::Constructor(va, name))
                    }
                    "destructure" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let pattern = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let value = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        let in_expr = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(4, &self))?;
                        Ok(Value::Destructure(va, pattern, value, in_expr))
                    }
                    "field" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let record = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        Ok(Value::Field(va, record, name))
                    }
                    "field_function" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(Value::FieldFunction(va, name))
                    }
                    "if_then_else" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let cond = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let then_branch = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        let else_branch = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(4, &self))?;
                        Ok(Value::IfThenElse(va, cond, then_branch, else_branch))
                    }
                    "lambda" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let pattern = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let body = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        Ok(Value::Lambda(va, pattern, body))
                    }
                    "let_definition" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let def = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        let in_expr = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(4, &self))?;
                        Ok(Value::LetDefinition(va, name, def, in_expr))
                    }
                    "let_recursion" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let defs = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let in_expr = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        Ok(Value::LetRecursion(va, defs, in_expr))
                    }
                    "list" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let elements = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(Value::List(va, elements))
                    }
                    "literal" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let lit = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(Value::Literal(va, lit))
                    }
                    "pattern_match" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let expr = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let cases = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        Ok(Value::PatternMatch(va, expr, cases))
                    }
                    "record" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let fields = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(Value::Record(va, fields))
                    }
                    "tuple" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let elements = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(Value::Tuple(va, elements))
                    }
                    "unit" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        Ok(Value::Unit(va))
                    }
                    "update" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let record = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let fields = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        Ok(Value::Update(va, record, fields))
                    }
                    "variable" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(Value::Variable(va, name))
                    }
                    _ => Err(de::Error::unknown_variant(
                        &tag,
                        &[
                            "apply",
                            "constructor",
                            "destructure",
                            "field",
                            "field_function",
                            "if_then_else",
                            "lambda",
                            "let_definition",
                            "let_recursion",
                            "list",
                            "literal",
                            "pattern_match",
                            "record",
                            "tuple",
                            "unit",
                            "update",
                            "variable",
                        ],
                    )),
                }
            }
        }
        deserializer.deserialize_seq(ValueVisitor(std::marker::PhantomData))
    }
}

/// Pattern for pattern matching
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern<A> {
    Wildcard(A),
    As(A, Box<Pattern<A>>, Name),
    Tuple(A, Vec<Pattern<A>>),
    Constructor(A, FQName, Vec<Pattern<A>>),
    EmptyList(A),
    HeadTail(A, Box<Pattern<A>>, Box<Pattern<A>>),
    Literal(A, Literal),
    Unit(A),
    Variable(A, Name),
}

impl<A: Serialize> Serialize for Pattern<A> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Pattern::Wildcard(a) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("wildcard")?;
                tuple.serialize_element(a)?;
                tuple.end()
            }
            Pattern::As(a, pattern, name) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("as_pattern")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(pattern)?;
                tuple.serialize_element(name)?;
                tuple.end()
            }
            Pattern::Tuple(a, patterns) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("tuple_pattern")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(patterns)?;
                tuple.end()
            }
            Pattern::Constructor(a, name, args) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("constructor_pattern")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(name)?;
                tuple.serialize_element(args)?;
                tuple.end()
            }
            Pattern::EmptyList(a) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("empty_list_pattern")?;
                tuple.serialize_element(a)?;
                tuple.end()
            }
            Pattern::HeadTail(a, head, tail) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("head_tail_pattern")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(head)?;
                tuple.serialize_element(tail)?;
                tuple.end()
            }
            Pattern::Literal(a, lit) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("literal_pattern")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(lit)?;
                tuple.end()
            }
            Pattern::Unit(a) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("unit_pattern")?;
                tuple.serialize_element(a)?;
                tuple.end()
            }
            Pattern::Variable(a, name) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("variable_pattern")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(name)?;
                tuple.end()
            }
        }
    }
}

impl<'de, A: Deserialize<'de>> Deserialize<'de> for Pattern<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PatternVisitor<A>(std::marker::PhantomData<A>);

        impl<'de, A: Deserialize<'de>> Visitor<'de> for PatternVisitor<A> {
            type Value = Pattern<A>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a classic Pattern array")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let tag: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;

                match tag.as_str() {
                    "wildcard" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        Ok(Pattern::Wildcard(a))
                    }
                    "as_pattern" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let pattern = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        Ok(Pattern::As(a, pattern, name))
                    }
                    "tuple_pattern" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let patterns = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(Pattern::Tuple(a, patterns))
                    }
                    "constructor_pattern" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let args = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        Ok(Pattern::Constructor(a, name, args))
                    }
                    "empty_list_pattern" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        Ok(Pattern::EmptyList(a))
                    }
                    "head_tail_pattern" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let head = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let tail = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        Ok(Pattern::HeadTail(a, head, tail))
                    }
                    "literal_pattern" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let lit = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(Pattern::Literal(a, lit))
                    }
                    "unit_pattern" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        Ok(Pattern::Unit(a))
                    }
                    "variable_pattern" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(Pattern::Variable(a, name))
                    }
                    _ => Err(de::Error::unknown_variant(
                        &tag,
                        &[
                            "wildcard",
                            "as_pattern",
                            "tuple_pattern",
                            "constructor_pattern",
                            "empty_list_pattern",
                            "head_tail_pattern",
                            "literal_pattern",
                            "unit_pattern",
                            "variable_pattern",
                        ],
                    )),
                }
            }
        }
        deserializer.deserialize_seq(PatternVisitor(std::marker::PhantomData))
    }
}

/// Value specification (inputs and output type)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueSpecification<A> {
    pub inputs: Vec<(Name, Type<A>)>,
    pub output: Type<A>,
}

/// Value definition (implementation)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueDefinition<TA, VA> {
    pub input_types: Vec<(Name, Type<TA>)>,
    pub output_type: Type<TA>,
    pub body: Value<TA, VA>,
}

/// Definition used in Let bindings
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Definition<TA, VA> {
    pub input_types: Vec<(Name, Type<TA>)>,
    pub output_type: Type<TA>,
    pub body: Value<TA, VA>,
}

/// Literal values
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Literal {
    Bool(bool),
    Char(char),
    String(String),
    Int(i64),
    Float(f64),
}

// ----------------------------------------------------------------------------
// Modules and Packages
// ----------------------------------------------------------------------------

/// Distribution of packages
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Distribution {
    pub format_version: u32,
    pub distribution: DistributionBody,
}

/// Distribution body
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DistributionBody {
    Library(LibraryTag, Path, Vec<serde_json::Value>, Package),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LibraryTag {
    #[serde(alias = "library")]
    Library,
}

/// Package definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Package {
    pub modules: Vec<Module>,
}

/// Module definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Module {
    pub name: Path,
    pub detail: ModuleDetail,
}

/// Module details
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleDetail {
    pub documentation: Option<String>,
    #[serde(default)]
    pub types: IndexMap<Name, AccessControlled<TypeDefinition<()>>>,
    #[serde(default)]
    pub values: IndexMap<Name, AccessControlled<ValueDefinition<(), ()>>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_serialize_type() {
        let t: Type<()> = Type::Variable((), Name::from("a"));
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, r#"["variable",null,["a"]]"#);
    }

    #[test]
    fn test_serialize_value_apply() {
        let v: Value<(), ()> = Value::Apply(
            (),
            Box::new(Value::Variable((), Name::from("f"))),
            Box::new(Value::Variable((), Name::from("x"))),
        );
        let json = serde_json::to_string(&v).unwrap();
        // Expect: ["apply", null, ["variable", null, ["f"]], ["variable", null, ["x"]]]
        assert!(json.contains("apply"));
        assert!(json.contains("variable"));
        assert!(json.contains("f"));
        assert!(json.contains("x"));
    }
}

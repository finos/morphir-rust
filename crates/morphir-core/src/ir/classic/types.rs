//! Classic IR Type system
//!
//! Type definitions for the Classic Morphir IR format (V1-V3 compatible).

use super::naming::{FQName, Name};
use serde::de::{self, SeqAccess, Visitor};
use serde::ser::{SerializeTuple, Serializer};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

use super::access::AccessControlled;

// ----------------------------------------------------------------------------
// Type Enum
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
                tuple.serialize_element("ExtensibleRecord")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(name)?;
                tuple.serialize_element(fields)?;
                tuple.end()
            }
            Type::Function(a, arg, ret) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("Function")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(arg)?;
                tuple.serialize_element(ret)?;
                tuple.end()
            }
            Type::Record(a, fields) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("Record")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(fields)?;
                tuple.end()
            }
            Type::Reference(a, name, args) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("Reference")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(name)?;
                tuple.serialize_element(args)?;
                tuple.end()
            }
            Type::Tuple(a, elements) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("Tuple")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(elements)?;
                tuple.end()
            }
            Type::Unit(a) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("Unit")?;
                tuple.serialize_element(a)?;
                tuple.end()
            }
            Type::Variable(a, name) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("Variable")?;
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
                
                // eprintln!("DEBUG: Type tag: {}", tag);

                match tag.as_str() {
                    "ExtensibleRecord" | "extensible_record" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let fields = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;

                        if let Some(_) = seq.next_element::<serde_json::Value>()? {
                             return Err(de::Error::custom("Expected end of ExtensibleRecord array"));
                        }

                        Ok(Type::ExtensibleRecord(a, name, fields))
                    }
                    "Function" | "function" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let arg = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let ret = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;

                        if let Some(_) = seq.next_element::<serde_json::Value>()? {
                             return Err(de::Error::custom("Expected end of Function array"));
                        }

                        Ok(Type::Function(a, arg, ret))
                    }
                    "Record" | "record" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let fields = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        
                        // Consume closing bracket
                        if let Some(_) = seq.next_element::<serde_json::Value>()? {
                             return Err(de::Error::custom("Expected end of Record array"));
                        }
                        
                        Ok(Type::Record(a, fields))
                    }
                    "Reference" | "reference" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let args = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        
                        // Consume closing bracket
                        if let Some(_) = seq.next_element::<serde_json::Value>()? {
                             return Err(de::Error::custom("Expected end of Reference array"));
                        }

                        Ok(Type::Reference(a, name, args))
                    }
                    "Tuple" | "tuple" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let elements = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;

                        if let Some(_) = seq.next_element::<serde_json::Value>()? {
                             return Err(de::Error::custom("Expected end of Tuple array"));
                        }

                        Ok(Type::Tuple(a, elements))
                    }
                    "Unit" | "unit" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        
                         if let Some(_) = seq.next_element::<serde_json::Value>()? {
                             return Err(de::Error::custom("Expected end of Unit array"));
                        }

                        Ok(Type::Unit(a))
                    }
                    "Variable" | "variable" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        
                        // Consume closing bracket
                        if let Some(_) = seq.next_element::<serde_json::Value>()? {
                             return Err(de::Error::custom("Expected end of Variable array"));
                        }

                        Ok(Type::Variable(a, name))
                    }
                    _ => Err(de::Error::unknown_variant(
                        &tag,
                        &[
                            "ExtensibleRecord",
                            "Function",
                            "Record",
                            "Reference",
                            "Tuple",
                            "Unit",
                            "Variable",
                        ],
                    )),
                }
            }
        }

        deserializer.deserialize_seq(TypeVisitor(std::marker::PhantomData))
    }
}

// ----------------------------------------------------------------------------
// Field
// ----------------------------------------------------------------------------

/// Record field definition - serialized as [name, type]
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
                
                if let Some(_) = seq.next_element::<serde_json::Value>()? {
                     return Err(de::Error::custom("Expected end of Field array"));
                }

                Ok(Field { name, tpe })
            }

            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
            where
                V: de::MapAccess<'de>,
            {
                let mut name = None;
                let mut tpe = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "name" => {
                            if name.is_some() {
                                return Err(de::Error::duplicate_field("name"));
                            }
                            name = Some(map.next_value()?);
                        }
                        "tpe" => {
                            if tpe.is_some() {
                                return Err(de::Error::duplicate_field("tpe"));
                            }
                            tpe = Some(map.next_value()?);
                        }
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>()?;
                        }
                    }
                }
                let name = name.ok_or_else(|| de::Error::missing_field("name"))?;
                let tpe = tpe.ok_or_else(|| de::Error::missing_field("tpe"))?;
                Ok(Field { name, tpe })
            }
        }
        deserializer.deserialize_any(FieldVisitor(std::marker::PhantomData))
    }
}

// ----------------------------------------------------------------------------
// TypeSpecification
// ----------------------------------------------------------------------------

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
                tuple.serialize_element("TypeAliasSpecification")?;
                tuple.serialize_element(params)?;
                tuple.serialize_element(tpe)?;
                tuple.end()
            }
            TypeSpecification::OpaqueTypeSpecification(params) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("OpaqueTypeSpecification")?;
                tuple.serialize_element(params)?;
                tuple.end()
            }
            TypeSpecification::CustomTypeSpecification(params, ctors) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("CustomTypeSpecification")?;
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
                    "TypeAliasSpecification" | "type_alias_specification" => {
                        let params = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let tpe = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        
                        if let Some(_) = seq.next_element::<serde_json::Value>()? {
                             return Err(de::Error::custom("Expected end of TypeAliasSpecification array"));
                        }

                        Ok(TypeSpecification::TypeAliasSpecification(params, tpe))
                    }
                    "OpaqueTypeSpecification" | "opaque_type_specification" => {
                        let params = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        
                        if let Some(_) = seq.next_element::<serde_json::Value>()? {
                             return Err(de::Error::custom("Expected end of OpaqueTypeSpecification array"));
                        }

                        Ok(TypeSpecification::OpaqueTypeSpecification(params))
                    }
                    "CustomTypeSpecification" | "custom_type_specification" => {
                        let params = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let ctors = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        
                        if let Some(_) = seq.next_element::<serde_json::Value>()? {
                             return Err(de::Error::custom("Expected end of CustomTypeSpecification array"));
                        }

                        Ok(TypeSpecification::CustomTypeSpecification(params, ctors))
                    }
                    _ => Err(de::Error::unknown_variant(
                        &tag,
                        &[
                            "TypeAliasSpecification",
                            "OpaqueTypeSpecification",
                            "CustomTypeSpecification",
                        ],
                    )),
                }
            }
        }
        deserializer.deserialize_seq(TSVisitor(std::marker::PhantomData))
    }
}

// ----------------------------------------------------------------------------
// Constructor
// ----------------------------------------------------------------------------

/// Constructor definition - [name, [args]]
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
                
                if let Some(_) = seq.next_element::<serde_json::Value>()? {
                     return Err(de::Error::custom("Expected end of Constructor array"));
                }

                Ok(Constructor { name, args })
            }
        }
        deserializer.deserialize_seq(CVisitor(std::marker::PhantomData))
    }
}

// ----------------------------------------------------------------------------
// TypeDefinition
// ----------------------------------------------------------------------------

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
                tuple.serialize_element("TypeAliasDefinition")?;
                tuple.serialize_element(params)?;
                tuple.serialize_element(tpe)?;
                tuple.end()
            }
            TypeDefinition::CustomTypeDefinition(params, ctors) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("CustomTypeDefinition")?;
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
                    "TypeAliasDefinition" | "type_alias_definition" => {
                        let params = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let tpe = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        
                        if let Some(_) = seq.next_element::<serde_json::Value>()? {
                             return Err(de::Error::custom("Expected end of TypeAliasDefinition array"));
                        }

                        Ok(TypeDefinition::TypeAliasDefinition(params, tpe))
                    }
                    "CustomTypeDefinition" | "custom_type_definition" => {
                        let params = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let ctors = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;

                        if let Some(_) = seq.next_element::<serde_json::Value>()? {
                             return Err(de::Error::custom("Expected end of CustomTypeDefinition array"));
                        }

                        Ok(TypeDefinition::CustomTypeDefinition(params, ctors))
                    }
                    _ => Err(de::Error::unknown_variant(
                        &tag,
                        &["TypeAliasDefinition", "CustomTypeDefinition"],
                    )),
                }
            }
        }
        deserializer.deserialize_seq(TDVisitor(std::marker::PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_type() {
        let t: Type<()> = Type::Variable((), Name::from_str("a"));
        let json = serde_json::to_string(&t).unwrap();
        // Name serializes as an array of lowercase words in Classic IR
        assert_eq!(json, r#"["Variable",null,["a"]]"#);
    }
}

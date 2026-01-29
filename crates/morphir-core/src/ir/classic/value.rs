//! Classic IR Value types
//!
//! Value expressions for the Classic Morphir IR format (V1-V3 compatible).

use super::naming::{FQName, Name};
use serde::de::{self, SeqAccess, Visitor};
use serde::ser::{SerializeTuple, Serializer};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

use super::literal::Literal;
use super::pattern::Pattern;
use super::types::Type;

// ----------------------------------------------------------------------------
// Value Enum
// ----------------------------------------------------------------------------

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
    LetDefinition(VA, Name, Box<Definition<TA, VA>>, Box<Value<TA, VA>>),
    LetRecursion(VA, Vec<(Name, Box<Definition<TA, VA>>)>, Box<Value<TA, VA>>),
    List(VA, Vec<Value<TA, VA>>),
    Literal(VA, Literal),
    PatternMatch(VA, Box<Value<TA, VA>>, Vec<(Pattern<VA>, Value<TA, VA>)>),
    Record(VA, Vec<(Name, Value<TA, VA>)>),
    Tuple(VA, Vec<Value<TA, VA>>),
    Unit(VA),
    Update(VA, Box<Value<TA, VA>>, Vec<(Name, Value<TA, VA>)>),
    Variable(VA, Name),
    Reference(VA, FQName),
}

impl<TA: Serialize, VA: Serialize> Serialize for Value<TA, VA> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Value::Apply(va, func, arg) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("Apply")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(func)?;
                tuple.serialize_element(arg)?;
                tuple.end()
            }
            Value::Constructor(va, name) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("Constructor")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(name)?;
                tuple.end()
            }
            Value::Destructure(va, pattern, value, in_expr) => {
                let mut tuple = serializer.serialize_tuple(5)?;
                tuple.serialize_element("Destructure")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(pattern)?;
                tuple.serialize_element(value)?;
                tuple.serialize_element(in_expr)?;
                tuple.end()
            }
            Value::Field(va, record, name) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("Field")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(record)?;
                tuple.serialize_element(name)?;
                tuple.end()
            }
            Value::FieldFunction(va, name) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("FieldFunction")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(name)?;
                tuple.end()
            }
            Value::IfThenElse(va, cond, then_branch, else_branch) => {
                let mut tuple = serializer.serialize_tuple(5)?;
                tuple.serialize_element("IfThenElse")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(cond)?;
                tuple.serialize_element(then_branch)?;
                tuple.serialize_element(else_branch)?;
                tuple.end()
            }
            Value::Lambda(va, pattern, body) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("Lambda")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(pattern)?;
                tuple.serialize_element(body)?;
                tuple.end()
            }
            Value::LetDefinition(va, name, def, in_expr) => {
                let mut tuple = serializer.serialize_tuple(5)?;
                tuple.serialize_element("LetDefinition")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(name)?;
                tuple.serialize_element(def)?;
                tuple.serialize_element(in_expr)?;
                tuple.end()
            }
            Value::LetRecursion(va, defs, in_expr) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("LetRecursion")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(defs)?;
                tuple.serialize_element(in_expr)?;
                tuple.end()
            }
            Value::List(va, elements) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("List")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(elements)?;
                tuple.end()
            }
            Value::Literal(va, lit) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("Literal")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(lit)?;
                tuple.end()
            }
            Value::PatternMatch(va, expr, cases) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("PatternMatch")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(expr)?;
                tuple.serialize_element(cases)?;
                tuple.end()
            }
            Value::Record(va, fields) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("Record")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(fields)?;
                tuple.end()
            }
            Value::Tuple(va, elements) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("Tuple")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(elements)?;
                tuple.end()
            }
            Value::Unit(va) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("Unit")?;
                tuple.serialize_element(va)?;
                tuple.end()
            }
            Value::Update(va, record, fields) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("Update")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(record)?;
                tuple.serialize_element(fields)?;
                tuple.end()
            }
            Value::Variable(va, name) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("Variable")?;
                tuple.serialize_element(va)?;
                tuple.serialize_element(name)?;
                tuple.end()
            }
            Value::Reference(va, name) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("Reference")?;
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
                    "Apply" | "apply" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let func = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let arg = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of Apply array")); }
                        Ok(Value::Apply(va, func, arg))
                    }
                    "Constructor" | "constructor" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of Constructor array")); }
                        Ok(Value::Constructor(va, name))
                    }
                    "Destructure" | "destructure" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let pattern = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let value = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        let in_expr = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(4, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of Destructure array")); }
                        Ok(Value::Destructure(va, pattern, value, in_expr))
                    }
                    "Field" | "field" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let record = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let name = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of Field array")); }
                        Ok(Value::Field(va, record, name))
                    }
                    "FieldFunction" | "field_function" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of FieldFunction array")); }
                        Ok(Value::FieldFunction(va, name))
                    }
                    "IfThenElse" | "if_then_else" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let cond = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let then_branch = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        let else_branch = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(4, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of IfThenElse array")); }
                        Ok(Value::IfThenElse(va, cond, then_branch, else_branch))
                    }
                    "Lambda" | "lambda" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let pattern = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let body = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of Lambda array")); }
                        Ok(Value::Lambda(va, pattern, body))
                    }
                    "LetDefinition" | "let_definition" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let def = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        let in_expr = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(4, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of LetDefinition array")); }
                        Ok(Value::LetDefinition(va, name, def, in_expr))
                    }
                    "LetRecursion" | "let_recursion" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let defs = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let in_expr = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of LetRecursion array")); }
                        Ok(Value::LetRecursion(va, defs, in_expr))
                    }
                    "List" | "list" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let elements = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of List array")); }
                        Ok(Value::List(va, elements))
                    }
                    "Literal" | "literal" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let lit = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of Literal array")); }
                        Ok(Value::Literal(va, lit))
                    }
                    "PatternMatch" | "pattern_match" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let expr = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let cases = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of PatternMatch array")); }
                        Ok(Value::PatternMatch(va, expr, cases))
                    }
                    "Record" | "record" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let fields = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of Record array")); }
                        Ok(Value::Record(va, fields))
                    }
                    "Tuple" | "tuple" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let elements = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of Tuple array")); }
                        Ok(Value::Tuple(va, elements))
                    }
                    "Unit" | "unit" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of Unit array")); }
                        Ok(Value::Unit(va))
                    }
                    "Update" | "update" | "UpdateRecord" | "update_record" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let record = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let fields = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of Update array")); }
                        Ok(Value::Update(va, record, fields))
                    }
                    "Variable" | "variable" => {
                        let va = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of Variable array")); }
                        Ok(Value::Variable(va, name))
                    }
                    "Reference" | "reference" => {
                        let va = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        if let Some(_) = seq.next_element::<serde_json::Value>()? { return Err(de::Error::custom("Expected end of Reference array")); }
                        Ok(Value::Reference(va, name))
                    }
                    _ => Err(de::Error::unknown_variant(&tag, &[
                        "Apply", "Constructor", "Destructure", "Field", "FieldFunction",
                        "IfThenElse", "Lambda", "LetDefinition", "LetRecursion", "List",
                        "Literal", "PatternMatch", "Record", "Tuple", "Unit", "Update", "Variable", "Reference",
                    ])),
                }
            }
        }
        deserializer.deserialize_seq(ValueVisitor(std::marker::PhantomData))
    }
}

// ----------------------------------------------------------------------------
// Value Specification and Definition
// ----------------------------------------------------------------------------

/// Value parameter - [name, type]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValueParameter<A> {
    pub name: Name,
    pub tpe: Type<A>,
}

impl<'de, A: Deserialize<'de>> Deserialize<'de> for ValueParameter<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct VPVisitor<A>(std::marker::PhantomData<A>);
        impl<'de, A: Deserialize<'de>> Visitor<'de> for VPVisitor<A> {
            type Value = ValueParameter<A>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a ValueParameter array [name, type]")
            }
            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let name = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let tpe = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                if let Some(_) = seq.next_element::<serde_json::Value>()? {
                    return Err(de::Error::custom("Expected end of ValueParameter array"));
                }
                Ok(ValueParameter { name, tpe })
            }
        }
        deserializer.deserialize_seq(VPVisitor(std::marker::PhantomData))
    }
}

/// Value argument - [name, va, type]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValueArgument<TA, VA> {
    pub name: Name,
    pub annotation: VA,
    pub tpe: Type<TA>,
}

impl<'de, TA: Deserialize<'de>, VA: Deserialize<'de>> Deserialize<'de> for ValueArgument<TA, VA> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct VAVisitor<TA, VA>(std::marker::PhantomData<(TA, VA)>);
        impl<'de, TA: Deserialize<'de>, VA: Deserialize<'de>> Visitor<'de> for VAVisitor<TA, VA> {
            type Value = ValueArgument<TA, VA>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a ValueArgument array [name, va, type]")
            }
            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let name = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let annotation = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let tpe = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
                if let Some(_) = seq.next_element::<serde_json::Value>()? {
                    return Err(de::Error::custom("Expected end of ValueArgument array"));
                }
                Ok(ValueArgument { name, annotation, tpe })
            }
        }
        deserializer.deserialize_seq(VAVisitor(std::marker::PhantomData))
    }
}

/// Value specification (inputs and output type)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueSpecification<A> {
    pub inputs: Vec<ValueParameter<A>>,
    pub output: Type<A>,
}

/// Value definition (implementation)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueDefinition<TA, VA> {
    pub input_types: Vec<ValueArgument<TA, VA>>,
    pub output_type: Type<TA>,
    pub body: Value<TA, VA>,
}

/// Definition used in Let bindings
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Definition<TA, VA> {
    pub input_types: Vec<ValueArgument<TA, VA>>,
    pub output_type: Type<TA>,
    pub body: Box<Value<TA, VA>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_value_apply() {
        let v: Value<(), ()> = Value::Apply(
            (),
            Box::new(Value::Variable((), Name::from_str("f"))),
            Box::new(Value::Variable((), Name::from_str("x"))),
        );
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("Apply"));
        assert!(json.contains("Variable"));
    }
}

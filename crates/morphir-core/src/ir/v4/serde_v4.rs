//! V4 object wrapper serialization for Morphir IR.
//!
//! V4 uses object wrapper format for all expressions:
//! - `{ "Variable": { "name": "a" } }` instead of `["Variable", {}, ["a"]]`
//! - `{ "Reference": { "fqname": "morphir/sdk:basics#int" } }` instead of `["Reference", {}, ...]`
//!
//! This module provides serialization helpers for Type, Pattern, Value,
//! and Literal using the V4 object wrapper format.

use indexmap::IndexMap;
use serde::ser::{SerializeMap, Serializer};
use serde::Serialize;

use super::attributes::{TypeAttributes, ValueAttributes};
use super::literal::Literal;
use super::pattern::Pattern;
use super::types::Type;
use super::value::{
    HoleReason, LetBinding, NativeInfo, PatternCase, RecordFieldEntry, Value, ValueDefinition,
};

// =============================================================================
// Type V4 Serialization
// =============================================================================

/// V4 serialization module for Type
pub mod type_serde {
    use super::*;

    /// Serialize Type in V4 object wrapper format
    pub fn serialize<S>(tpe: &Type, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_type(tpe, serializer)
    }
}

/// Serialize a Type in V4 object wrapper format
pub fn serialize_type<S>(tpe: &Type, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match tpe {
        Type::Variable(attrs, name) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Variable",
                &VariableContent {
                    name: name.to_string(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Type::Reference(attrs, fqname, args) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Reference",
                &ReferenceContent {
                    fqname: fqname.to_canonical_string(),
                    args,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Type::Tuple(attrs, elements) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Tuple",
                &TupleContent {
                    elements,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Type::Record(attrs, fields) => {
            let mut map = serializer.serialize_map(Some(1))?;
            // V4 Record fields are object: { "fieldName": typeExpr }
            let fields_map: IndexMap<String, &Type> = fields
                .iter()
                .map(|f| (f.name.to_string(), &f.tpe))
                .collect();
            map.serialize_entry(
                "Record",
                &RecordContent {
                    fields: fields_map,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Type::ExtensibleRecord(attrs, var, fields) => {
            let mut map = serializer.serialize_map(Some(1))?;
            let fields_map: IndexMap<String, &Type> = fields
                .iter()
                .map(|f| (f.name.to_string(), &f.tpe))
                .collect();
            map.serialize_entry(
                "ExtensibleRecord",
                &ExtensibleRecordContent {
                    variable: var.to_string(),
                    fields: fields_map,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Type::Function(attrs, arg, result) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Function",
                &FunctionContent {
                    arg: arg.as_ref(),
                    result: result.as_ref(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Type::Unit(attrs) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("Unit", &UnitContent { attrs: Some(attrs) })?;
            map.end()
        }
    }
}

// Helper structs for V4 Type serialization

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VariableContent<'a> {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a TypeAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceContent<'a> {
    fqname: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    args: &'a Vec<Type>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a TypeAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TupleContent<'a> {
    elements: &'a Vec<Type>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a TypeAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordContent<'a> {
    fields: IndexMap<String, &'a Type>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a TypeAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensibleRecordContent<'a> {
    variable: String,
    fields: IndexMap<String, &'a Type>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a TypeAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FunctionContent<'a> {
    arg: &'a Type,
    result: &'a Type,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a TypeAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UnitContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a TypeAttributes>,
}

// =============================================================================
// Literal V4 Serialization
// =============================================================================

/// Serialize Literal in V4 object wrapper format
pub fn serialize_literal<S>(lit: &Literal, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match lit {
        Literal::Bool(v) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("BoolLiteral", &LiteralValue { value: v })?;
            map.end()
        }
        Literal::Char(v) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "CharLiteral",
                &LiteralValue {
                    value: v.to_string(),
                },
            )?;
            map.end()
        }
        Literal::String(v) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("StringLiteral", &LiteralValue { value: v })?;
            map.end()
        }
        Literal::Integer(v) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("IntegerLiteral", &LiteralValue { value: v })?;
            map.end()
        }
        Literal::Float(v) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("FloatLiteral", &LiteralValue { value: v })?;
            map.end()
        }
        Literal::Decimal(v) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("DecimalLiteral", &LiteralValue { value: v })?;
            map.end()
        }
    }
}

#[derive(Serialize)]
struct LiteralValue<T: Serialize> {
    value: T,
}

// =============================================================================
// Pattern V4 Serialization
// =============================================================================

/// V4 serialization module for Pattern
pub mod pattern_serde {
    use super::*;

    /// Serialize Pattern in V4 object wrapper format
    pub fn serialize<S>(pat: &Pattern, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_pattern(pat, serializer)
    }
}

/// Serialize a Pattern in V4 object wrapper format
pub fn serialize_pattern<S>(pat: &Pattern, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match pat {
        Pattern::WildcardPattern(attrs) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "WildcardPattern",
                &PatternAttrsContent { attrs: Some(attrs) },
            )?;
            map.end()
        }
        Pattern::AsPattern(attrs, pattern, name) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "AsPattern",
                &AsPatternContent {
                    pattern,
                    name: name.to_string(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Pattern::TuplePattern(attrs, elements) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "TuplePattern",
                &TuplePatternContent {
                    elements,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Pattern::ConstructorPattern(attrs, fqname, args) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "ConstructorPattern",
                &ConstructorPatternContent {
                    fqname: fqname.to_canonical_string(),
                    args,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Pattern::EmptyListPattern(attrs) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "EmptyListPattern",
                &PatternAttrsContent { attrs: Some(attrs) },
            )?;
            map.end()
        }
        Pattern::HeadTailPattern(attrs, head, tail) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "HeadTailPattern",
                &HeadTailPatternContent {
                    head: head.as_ref(),
                    tail: tail.as_ref(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Pattern::LiteralPattern(attrs, lit) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "LiteralPattern",
                &LiteralPatternContent {
                    literal: lit,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Pattern::UnitPattern(attrs) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("UnitPattern", &PatternAttrsContent { attrs: Some(attrs) })?;
            map.end()
        }
    }
}

// Helper structs for V4 Pattern serialization

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PatternAttrsContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AsPatternContent<'a> {
    #[serde(serialize_with = "serialize_pattern")]
    pattern: &'a Pattern,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TuplePatternContent<'a> {
    elements: &'a Vec<Pattern>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConstructorPatternContent<'a> {
    fqname: String,
    args: &'a Vec<Pattern>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HeadTailPatternContent<'a> {
    #[serde(serialize_with = "serialize_pattern")]
    head: &'a Pattern,
    #[serde(serialize_with = "serialize_pattern")]
    tail: &'a Pattern,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LiteralPatternContent<'a> {
    #[serde(serialize_with = "serialize_literal")]
    literal: &'a Literal,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

// =============================================================================
// Value V4 Serialization
// =============================================================================

/// V4 serialization module for Value
pub mod value_serde {
    use super::*;

    /// Serialize Value in V4 object wrapper format
    pub fn serialize<S>(val: &Value, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_value(val, serializer)
    }
}

/// Deserialize Value from V4 format (uses the impl in serde_tagged.rs)
pub fn deserialize_value<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(deserializer)
}

/// Serialize a Value in V4 object wrapper format
pub fn serialize_value<S>(val: &Value, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match val {
        Value::Literal(attrs, lit) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Literal",
                &LiteralValueContent {
                    literal: lit,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Constructor(attrs, fqname) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Constructor",
                &ConstructorValueContent {
                    fqname: fqname.to_canonical_string(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Tuple(attrs, elements) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Tuple",
                &TupleValueContent {
                    elements,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::List(attrs, items) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "List",
                &ListValueContent {
                    items,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Record(attrs, fields) => {
            let mut map = serializer.serialize_map(Some(1))?;
            let fields_map: IndexMap<String, &Value> = fields
                .iter()
                .map(|f| (f.0.to_string(), &f.1))
                .collect();
            map.serialize_entry(
                "Record",
                &RecordValueContent {
                    fields: fields_map,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Variable(attrs, name) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Variable",
                &VariableValueContent {
                    name: name.to_string(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Reference(attrs, fqname) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Reference",
                &ReferenceValueContent {
                    fqname: fqname.to_canonical_string(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Field(attrs, value, name) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Field",
                &FieldValueContent {
                    value: value.as_ref(),
                    name: name.to_string(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::FieldFunction(attrs, name) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "FieldFunction",
                &FieldFunctionContent {
                    name: name.to_string(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Apply(attrs, function, argument) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Apply",
                &ApplyContent {
                    function: function.as_ref(),
                    argument: argument.as_ref(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Lambda(attrs, pattern, body) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Lambda",
                &LambdaContent {
                    pattern,
                    body: body.as_ref(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::LetDefinition(attrs, name, definition, body) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "LetDefinition",
                &LetDefinitionContent {
                    name: name.to_string(),
                    definition: definition.as_ref(),
                    body: body.as_ref(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::LetRecursion(attrs, bindings, body) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "LetRecursion",
                &LetRecursionContent {
                    bindings,
                    body: body.as_ref(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Destructure(attrs, pattern, value, body) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Destructure",
                &DestructureContent {
                    pattern,
                    value: value.as_ref(),
                    body: body.as_ref(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::IfThenElse(attrs, condition, then_branch, else_branch) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "IfThenElse",
                &IfThenElseContent {
                    condition: condition.as_ref(),
                    then_branch: then_branch.as_ref(),
                    else_branch: else_branch.as_ref(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::PatternMatch(attrs, subject, cases) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "PatternMatch",
                &PatternMatchContent {
                    subject: subject.as_ref(),
                    cases,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::UpdateRecord(attrs, record, updates) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "UpdateRecord",
                &UpdateRecordContent {
                    record: record.as_ref(),
                    updates,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Unit(attrs) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("Unit", &ValueUnitContent { attrs: Some(attrs) })?;
            map.end()
        }
        Value::Hole(attrs, reason, tpe) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Hole",
                &HoleContent {
                    reason,
                    tpe: tpe.as_ref().map(|t: &Box<_>| t.as_ref()),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::Native(attrs, fqname, info) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Native",
                &NativeValueContent {
                    fqname: fqname.to_canonical_string(),
                    info,
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
        Value::External(attrs, external_name, target_platform) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "External",
                &ExternalContent {
                    external_name: external_name.clone(),
                    target_platform: target_platform.clone(),
                    attrs: Some(attrs),
                },
            )?;
            map.end()
        }
    }
}

// Helper structs for V4 Value serialization

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LiteralValueContent<'a> {
    #[serde(serialize_with = "serialize_literal")]
    literal: &'a Literal,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConstructorValueContent<'a> {
    fqname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TupleValueContent<'a> {
    elements: &'a Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListValueContent<'a> {
    items: &'a Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordValueContent<'a> {
    fields: IndexMap<String, &'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VariableValueContent<'a> {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceValueContent<'a> {
    fqname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldValueContent<'a> {
    #[serde(serialize_with = "serialize_value")]
    value: &'a Value,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldFunctionContent<'a> {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApplyContent<'a> {
    #[serde(serialize_with = "serialize_value")]
    function: &'a Value,
    #[serde(serialize_with = "serialize_value")]
    argument: &'a Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LambdaContent<'a> {
    #[serde(serialize_with = "serialize_pattern")]
    pattern: &'a Pattern,
    #[serde(serialize_with = "serialize_value")]
    body: &'a Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LetDefinitionContent<'a> {
    name: String,
    definition: &'a ValueDefinition,
    #[serde(serialize_with = "serialize_value")]
    body: &'a Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LetRecursionContent<'a> {
    bindings: &'a Vec<LetBinding>,
    #[serde(serialize_with = "serialize_value")]
    body: &'a Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DestructureContent<'a> {
    #[serde(serialize_with = "serialize_pattern")]
    pattern: &'a Pattern,
    #[serde(serialize_with = "serialize_value")]
    value: &'a Value,
    #[serde(serialize_with = "serialize_value")]
    body: &'a Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IfThenElseContent<'a> {
    #[serde(serialize_with = "serialize_value")]
    condition: &'a Value,
    #[serde(serialize_with = "serialize_value")]
    then_branch: &'a Value,
    #[serde(serialize_with = "serialize_value")]
    else_branch: &'a Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PatternMatchContent<'a> {
    #[serde(serialize_with = "serialize_value")]
    subject: &'a Value,
    cases: &'a Vec<PatternCase>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateRecordContent<'a> {
    #[serde(serialize_with = "serialize_value")]
    record: &'a Value,
    updates: &'a Vec<RecordFieldEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ValueUnitContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HoleContent<'a> {
    reason: &'a HoleReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    tpe: Option<&'a Type>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeValueContent<'a> {
    fqname: String,
    info: &'a NativeInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalContent<'a> {
    external_name: String,
    target_platform: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    attrs: Option<&'a ValueAttributes>,
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::naming::Name;

    #[test]
    fn test_serialize_type_variable() {
        let var = Type::Variable(TypeAttributes::default(), Name::from("a"));
        let json = serde_json::to_string(&var).unwrap();
        assert!(json.contains("\"Variable\""));
        assert!(json.contains("\"name\""));
    }

    #[test]
    fn test_serialize_pattern_wildcard() {
        let p = Pattern::WildcardPattern(ValueAttributes::default());
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"WildcardPattern\""));
    }

    #[test]
    fn test_serialize_value_unit() {
        let v = Value::Unit(ValueAttributes::default());
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("\"Unit\""));
    }
}

//! V4 object wrapper serialization for Morphir IR.
//!
//! Every expression is a single-member wrapper rather than a Classic tagged array:
//! `{ "Variable": { "name": "a" } }` instead of `["Variable", {}, ["a"]]`.
//!
//! For type expressions [`TypeEncoding::Compact`] is the canonical spelling. It writes the
//! shortest form a reader accepts — `"a"` for a variable, `"morphir/SDK:basics#int"` for a
//! reference with no arguments — and falls back to the expanded spelling, whose payload opens
//! with `attributes`, exactly when a node carries attributes worth writing (decision 0005).

use indexmap::IndexMap;
use serde::Serialize;
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use std::cell::Cell;

use super::attributes::{TypeAttributes, ValueAttributes};
use super::literal::Literal;
use super::pattern::Pattern;
use super::types::Type;
use super::value::{HoleReason, RecordFieldEntry, Value, ValueDefinition};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TypeEncoding {
    Compact,
    Expanded,
}

thread_local! {
    static TYPE_ENCODING: Cell<TypeEncoding> = const { Cell::new(TypeEncoding::Expanded) };
}

/// Select the v4 type encoding for every nested type serialized by `operation`.
pub fn with_type_encoding<R>(encoding: TypeEncoding, operation: impl FnOnce() -> R) -> R {
    struct Restore(TypeEncoding);
    impl Drop for Restore {
        fn drop(&mut self) {
            TYPE_ENCODING.set(self.0);
        }
    }

    let previous = TYPE_ENCODING.replace(encoding);
    let _restore = Restore(previous);
    operation()
}

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
    if TYPE_ENCODING.get() == TypeEncoding::Compact
        && tpe.attributes() == &TypeAttributes::default()
    {
        return serialize_compact_type(tpe, serializer);
    }
    match tpe {
        Type::Variable(attrs, name) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Variable",
                &VariableContent {
                    attributes: written(attrs),
                    name: name.to_canonical_string(),
                },
            )?;
            map.end()
        }
        Type::Reference(attrs, fqname, args) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Reference",
                &ReferenceContent {
                    attributes: written(attrs),
                    fqname: fqname.to_canonical_string(),
                    args,
                },
            )?;
            map.end()
        }
        Type::Tuple(attrs, elements) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Tuple",
                &TupleContent {
                    attributes: written(attrs),
                    elements,
                },
            )?;
            map.end()
        }
        Type::Record(attrs, fields) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Record",
                &RecordContent {
                    attributes: written(attrs),
                    fields: field_map(fields),
                },
            )?;
            map.end()
        }
        Type::ExtensibleRecord(attrs, var, fields) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "ExtensibleRecord",
                &ExtensibleRecordContent {
                    attributes: written(attrs),
                    variable: var.to_canonical_string(),
                    fields: field_map(fields),
                },
            )?;
            map.end()
        }
        Type::Function(attrs, arg, result) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Function",
                &FunctionContent {
                    attributes: written(attrs),
                    parameter_type: arg.as_ref(),
                    return_type: result.as_ref(),
                },
            )?;
            map.end()
        }
        Type::Unit(attrs) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Unit",
                &UnitContent {
                    attributes: written(attrs),
                },
            )?;
            map.end()
        }
    }
}

/// Decision 0005: an empty `attributes` member is accepted but never written.
fn written(attrs: &TypeAttributes) -> Option<&TypeAttributes> {
    (attrs != &TypeAttributes::default()).then_some(attrs)
}

/// A record's fields are an object keyed by field name, so the declaration order is the
/// order of the members.
fn field_map(fields: &[crate::ir::v4::types::Field]) -> IndexMap<String, &Type> {
    fields
        .iter()
        .map(|field| (field.name.to_canonical_string(), &field.tpe))
        .collect()
}

// Helper structs for V4 Type serialization

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VariableContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a TypeAttributes>,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a TypeAttributes>,
    fqname: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    args: &'a Vec<Type>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TupleContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a TypeAttributes>,
    elements: &'a Vec<Type>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a TypeAttributes>,
    fields: IndexMap<String, &'a Type>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensibleRecordContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a TypeAttributes>,
    variable: String,
    fields: IndexMap<String, &'a Type>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FunctionContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a TypeAttributes>,
    parameter_type: &'a Type,
    return_type: &'a Type,
}

struct ReferenceArgs<'a> {
    fqname: String,
    args: &'a [Type],
}

impl Serialize for ReferenceArgs<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.args.len() + 1))?;
        sequence.serialize_element(&self.fqname)?;
        for argument in self.args {
            sequence.serialize_element(argument)?;
        }
        sequence.end()
    }
}

fn serialize_compact_type<S>(tpe: &Type, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match tpe {
        Type::Variable(_, name) => serializer.serialize_str(&name.to_canonical_string()),
        Type::Reference(_, fqname, arguments) if arguments.is_empty() => {
            serializer.serialize_str(&fqname.to_canonical_string())
        }
        Type::Reference(_, fqname, arguments) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Reference",
                &ReferenceArgs {
                    fqname: fqname.to_canonical_string(),
                    args: arguments,
                },
            )?;
            map.end()
        }
        Type::Tuple(_, elements) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("Tuple", elements)?;
            map.end()
        }
        Type::Record(_, fields) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Record",
                &RecordContent {
                    attributes: None,
                    fields: field_map(fields),
                },
            )?;
            map.end()
        }
        Type::ExtensibleRecord(_, variable, fields) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "ExtensibleRecord",
                &ExtensibleRecordContent {
                    attributes: None,
                    variable: variable.to_canonical_string(),
                    fields: field_map(fields),
                },
            )?;
            map.end()
        }
        Type::Function(_, argument, result) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(
                "Function",
                &FunctionContent {
                    attributes: None,
                    parameter_type: argument,
                    return_type: result,
                },
            )?;
            map.end()
        }
        Type::Unit(_) => {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("Unit", &IndexMap::<String, String>::new())?;
            map.end()
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UnitContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a TypeAttributes>,
}

// =============================================================================
// Literal V4 Serialization
// =============================================================================

/// Serialize a Literal in its canonical v4 spelling: the payload sits directly under the tag,
/// so `{ "IntegerLiteral": 42 }` rather than `{ "IntegerLiteral": { "value": 42 } }`.
pub fn serialize_literal<S>(lit: &Literal, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut map = serializer.serialize_map(Some(1))?;
    match lit {
        Literal::Bool(v) => map.serialize_entry("BoolLiteral", v)?,
        Literal::Char(v) => map.serialize_entry("CharLiteral", &v.to_string())?,
        Literal::String(v) => map.serialize_entry("StringLiteral", v)?,
        Literal::Integer(v) => map.serialize_entry("IntegerLiteral", v)?,
        // A float is written from the number it was read as, so the spelling survives the round
        // trip: `1.0e2` comes back out as `1.0e2`, and a float written `4` comes back out as `4`.
        // The tag is what says it is a float, so an integral spelling loses nothing. With
        // `arbitrary_precision`, a `Number` built from text serializes as that text.
        Literal::Float(v) => map.serialize_entry("FloatLiteral", v.number())?,
        Literal::Decimal(v) => map.serialize_entry("DecimalLiteral", v)?,
        Literal::Document(v) => map.serialize_entry("DocumentLiteral", v)?,
    }
    map.end()
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

/// Serialize a Pattern in its canonical v4 spelling.
///
/// A tuple is an array of patterns and a literal pattern carries the literal directly, so a
/// pattern that has nothing to say about itself is as short as the reader allows. Decision 0005
/// keeps an empty `attributes` unwritten; a pattern that does carry attributes writes the
/// expanded spelling instead, `attributes` first.
pub fn serialize_pattern<S>(pat: &Pattern, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let attributes = written_value(pat.attributes());
    let mut map = serializer.serialize_map(Some(1))?;
    match pat {
        Pattern::WildcardPattern(_) => {
            map.serialize_entry("WildcardPattern", &PatternAttributes { attributes })?
        }
        Pattern::EmptyListPattern(_) => {
            map.serialize_entry("EmptyListPattern", &PatternAttributes { attributes })?
        }
        Pattern::UnitPattern(_) => {
            map.serialize_entry("UnitPattern", &PatternAttributes { attributes })?
        }
        Pattern::AsPattern(_, pattern, name) => map.serialize_entry(
            "AsPattern",
            &AsPatternContent {
                attributes,
                pattern,
                name: name.to_canonical_string(),
            },
        )?,
        Pattern::TuplePattern(_, patterns) => match attributes {
            None => map.serialize_entry("TuplePattern", patterns)?,
            Some(_) => map.serialize_entry(
                "TuplePattern",
                &TuplePatternContent {
                    attributes,
                    patterns,
                },
            )?,
        },
        Pattern::ConstructorPattern(_, fqname, patterns) => map.serialize_entry(
            "ConstructorPattern",
            &ConstructorPatternContent {
                attributes,
                fqname: fqname.to_canonical_string(),
                patterns,
            },
        )?,
        Pattern::HeadTailPattern(_, head, tail) => map.serialize_entry(
            "HeadTailPattern",
            &HeadTailPatternContent {
                attributes,
                head: head.as_ref(),
                tail: tail.as_ref(),
            },
        )?,
        Pattern::LiteralPattern(_, lit) => match attributes {
            None => map.serialize_entry("LiteralPattern", lit)?,
            Some(_) => map.serialize_entry(
                "LiteralPattern",
                &LiteralPatternContent {
                    attributes,
                    literal: lit,
                },
            )?,
        },
    }
    map.end()
}

/// Decision 0005: an empty `attributes` member is accepted but never written.
fn written_value(attrs: &ValueAttributes) -> Option<&ValueAttributes> {
    (attrs != &ValueAttributes::default()).then_some(attrs)
}

// Helper structs for V4 Pattern serialization. `attributes` is declared first in each, which is
// the order the expanded spelling writes it in.

#[derive(Serialize)]
struct PatternAttributes<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
struct AsPatternContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    #[serde(serialize_with = "serialize_pattern")]
    pattern: &'a Pattern,
    name: String,
}

#[derive(Serialize)]
struct TuplePatternContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    patterns: &'a Vec<Pattern>,
}

#[derive(Serialize)]
struct ConstructorPatternContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    fqname: String,
    patterns: &'a Vec<Pattern>,
}

#[derive(Serialize)]
struct HeadTailPatternContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    #[serde(serialize_with = "serialize_pattern")]
    head: &'a Pattern,
    #[serde(serialize_with = "serialize_pattern")]
    tail: &'a Pattern,
}

#[derive(Serialize)]
struct LiteralPatternContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    #[serde(serialize_with = "serialize_literal")]
    literal: &'a Literal,
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

/// Serialize a Value in its canonical v4 spelling.
///
/// Decision 0009's shorthands are what a writer uses when a node has nothing else to say about
/// itself: a variable is its name, a reference and a constructor are their FQName, a list and a
/// tuple are their items, and a literal carries its payload directly. Decision 0005 keeps an
/// empty `attributes` unwritten; a node that does carry attributes writes the expanded spelling
/// instead, `attributes` first.
pub fn serialize_value<S>(val: &Value, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let attributes = written_value(val.attributes());
    let mut map = serializer.serialize_map(Some(1))?;
    match val {
        Value::Literal(_, literal) => match attributes {
            None => map.serialize_entry("Literal", literal)?,
            Some(_) => map.serialize_entry(
                "Literal",
                &LiteralValueContent {
                    attributes,
                    literal,
                },
            )?,
        },
        Value::Constructor(_, fqname) => match attributes {
            None => map.serialize_entry("Constructor", &fqname.to_canonical_string())?,
            Some(_) => map.serialize_entry(
                "Constructor",
                &FqNameValueContent {
                    attributes,
                    fqname: fqname.to_canonical_string(),
                },
            )?,
        },
        Value::Reference(_, fqname) => match attributes {
            None => map.serialize_entry("Reference", &fqname.to_canonical_string())?,
            Some(_) => map.serialize_entry(
                "Reference",
                &FqNameValueContent {
                    attributes,
                    fqname: fqname.to_canonical_string(),
                },
            )?,
        },
        Value::Variable(_, name) => match attributes {
            None => map.serialize_entry("Variable", &name.to_canonical_string())?,
            Some(_) => map.serialize_entry(
                "Variable",
                &NamedValueContent {
                    attributes,
                    name: name.to_canonical_string(),
                },
            )?,
        },
        Value::FieldFunction(_, name) => match attributes {
            None => map.serialize_entry("FieldFunction", &name.to_canonical_string())?,
            Some(_) => map.serialize_entry(
                "FieldFunction",
                &NamedValueContent {
                    attributes,
                    name: name.to_canonical_string(),
                },
            )?,
        },
        Value::Tuple(_, elements) => match attributes {
            None => map.serialize_entry("Tuple", elements)?,
            Some(_) => map.serialize_entry(
                "Tuple",
                &TupleValueContent {
                    attributes,
                    elements,
                },
            )?,
        },
        Value::List(_, items) => match attributes {
            None => map.serialize_entry("List", items)?,
            Some(_) => map.serialize_entry("List", &ListValueContent { attributes, items })?,
        },
        Value::Record(_, fields) => map.serialize_entry(
            "Record",
            &RecordValueContent {
                attributes,
                fields: field_values(fields),
            },
        )?,
        Value::Field(_, target, name) => map.serialize_entry(
            "Field",
            &FieldValueContent {
                attributes,
                target: target.as_ref(),
                name: name.to_canonical_string(),
            },
        )?,
        Value::Apply(_, function, argument) => map.serialize_entry(
            "Apply",
            &ApplyContent {
                attributes,
                function: function.as_ref(),
                argument: argument.as_ref(),
            },
        )?,
        Value::Lambda(_, pattern, body) => map.serialize_entry(
            "Lambda",
            &LambdaContent {
                attributes,
                pattern,
                body: body.as_ref(),
            },
        )?,
        Value::LetDefinition(_, name, definition, body) => map.serialize_entry(
            "LetDefinition",
            &LetDefinitionContent {
                attributes,
                name: name.to_canonical_string(),
                definition: definition.as_ref(),
                body: body.as_ref(),
            },
        )?,
        Value::LetRecursion(_, bindings, body) => map.serialize_entry(
            "LetRecursion",
            &LetRecursionContent {
                attributes,
                definitions: bindings
                    .iter()
                    .map(|binding| (binding.name().to_canonical_string(), binding.definition()))
                    .collect(),
                body: body.as_ref(),
            },
        )?,
        Value::Destructure(_, pattern, value, body) => map.serialize_entry(
            "Destructure",
            &DestructureContent {
                attributes,
                pattern,
                value: value.as_ref(),
                body: body.as_ref(),
            },
        )?,
        Value::IfThenElse(_, condition, then_branch, else_branch) => map.serialize_entry(
            "IfThenElse",
            &IfThenElseContent {
                attributes,
                condition: condition.as_ref(),
                then_branch: then_branch.as_ref(),
                else_branch: else_branch.as_ref(),
            },
        )?,
        Value::PatternMatch(_, value, cases) => map.serialize_entry(
            "PatternMatch",
            &PatternMatchContent {
                attributes,
                value: value.as_ref(),
                cases: cases
                    .iter()
                    .map(|case| PatternCaseContent {
                        pattern: case.pattern(),
                        body: case.body(),
                    })
                    .collect(),
            },
        )?,
        Value::UpdateRecord(_, target, fields) => map.serialize_entry(
            "UpdateRecord",
            &UpdateRecordContent {
                attributes,
                target: target.as_ref(),
                fields: field_values(fields),
            },
        )?,
        Value::Unit(_) => map.serialize_entry("Unit", &ValueUnitContent { attributes })?,
        Value::Hole(_, reason, expected_type) => map.serialize_entry(
            "Hole",
            &HoleContent {
                attributes,
                reason,
                expected_type: expected_type.as_ref().map(|tpe| tpe.as_ref()),
            },
        )?,
    }
    map.end()
}

/// A record's fields keep their declaration order, which is the order the entries carry.
fn field_values(fields: &[RecordFieldEntry]) -> IndexMap<String, &Value> {
    fields
        .iter()
        .map(|field| (field.name().to_canonical_string(), field.value()))
        .collect()
}

// Helper structs for V4 Value serialization. `attributes` is declared first in each, which is
// the order the expanded spelling writes it in.

#[derive(Serialize)]
struct LiteralValueContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    literal: &'a Literal,
}

#[derive(Serialize)]
struct FqNameValueContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    fqname: String,
}

#[derive(Serialize)]
struct NamedValueContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    name: String,
}

#[derive(Serialize)]
struct TupleValueContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    elements: &'a Vec<Value>,
}

#[derive(Serialize)]
struct ListValueContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    items: &'a Vec<Value>,
}

#[derive(Serialize)]
struct RecordValueContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    fields: IndexMap<String, &'a Value>,
}

#[derive(Serialize)]
struct FieldValueContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    target: &'a Value,
    name: String,
}

#[derive(Serialize)]
struct ApplyContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    function: &'a Value,
    argument: &'a Value,
}

#[derive(Serialize)]
struct LambdaContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    pattern: &'a Pattern,
    body: &'a Value,
}

#[derive(Serialize)]
struct LetDefinitionContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    name: String,
    definition: &'a ValueDefinition,
    #[serde(rename = "in")]
    body: &'a Value,
}

#[derive(Serialize)]
struct LetRecursionContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    definitions: IndexMap<String, &'a ValueDefinition>,
    #[serde(rename = "in")]
    body: &'a Value,
}

#[derive(Serialize)]
struct DestructureContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    pattern: &'a Pattern,
    value: &'a Value,
    #[serde(rename = "in")]
    body: &'a Value,
}

#[derive(Serialize)]
struct IfThenElseContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    condition: &'a Value,
    #[serde(rename = "then")]
    then_branch: &'a Value,
    #[serde(rename = "else")]
    else_branch: &'a Value,
}

#[derive(Serialize)]
struct PatternMatchContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    value: &'a Value,
    cases: Vec<PatternCaseContent<'a>>,
}

/// A pattern match case is an object, not a pair: `{ "pattern": …, "body": … }`.
#[derive(Serialize)]
struct PatternCaseContent<'a> {
    pattern: &'a Pattern,
    body: &'a Value,
}

#[derive(Serialize)]
struct UpdateRecordContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    target: &'a Value,
    fields: IndexMap<String, &'a Value>,
}

#[derive(Serialize)]
struct ValueUnitContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HoleContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<&'a ValueAttributes>,
    reason: &'a HoleReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_type: Option<&'a Type>,
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

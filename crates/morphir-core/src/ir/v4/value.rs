//! Value expressions and definitions for Morphir IR V4.
//!
//! This module defines the complete value layer of the Morphir IR, including:
//! - Value expressions (`Value` enum) representing term-level computations
//! - Value specifications (`ValueSpecification`) representing type signatures
//! - Value definitions (`ValueDefinition`) with various body types
//!
//! Values use `TypeAttributes` for type nodes and `ValueAttributes` for value nodes (V4 format).
//!
//! # Examples
//!
//! ```rust,ignore
//! // Create a simple unit value
//! let v: Value = Value::Unit(ValueAttributes::default());
//!
//! // Create a value definition
//! let def: ValueDefinition = ValueDefinition::new(
//!     vec![],
//!     Type::unit(TypeAttributes::default()),
//!     Value::unit(ValueAttributes::default()),
//! );
//! ```

use indexmap::IndexMap;
use serde::de::{self, Deserializer};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};

use super::attributes::ValueAttributes;
use super::literal::Literal;
use super::pattern::Pattern;
use super::types::Type;
use crate::naming::{FQName, Name};

// ============================================================================
// VALUE EXPRESSIONS
// ============================================================================

/// A value expression with V4 attributes.
///
/// Value expressions form the term-level representation in Morphir IR.
/// Each variant carries `ValueAttributes`, and types within
/// carry `TypeAttributes`.
///
/// # Examples
///
/// ```rust,ignore
/// let v: Value = Value::Unit(ValueAttributes::default());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    // === Core expressions (all versions) ===
    /// Literal constant value
    ///
    /// Example: `42`, `"hello"`, `true`
    Literal(ValueAttributes, Literal),

    /// Data constructor reference
    ///
    /// Example: `Just` in `Just 42`
    Constructor(ValueAttributes, FQName),

    /// Tuple construction
    ///
    /// Example: `(1, "hello", true)`
    Tuple(ValueAttributes, Vec<Value>),

    /// List construction
    ///
    /// Example: `[1, 2, 3]`
    List(ValueAttributes, Vec<Value>),

    /// Record construction
    ///
    /// Example: `{ name = "Alice", age = 30 }`
    Record(ValueAttributes, Vec<RecordFieldEntry>),

    /// Variable reference
    ///
    /// Example: `x` in `let x = 1 in x + 1`
    Variable(ValueAttributes, Name),

    /// Reference to a named value
    ///
    /// Example: `List.map` referencing a module function
    Reference(ValueAttributes, FQName),

    /// Field access on a record
    ///
    /// Example: `person.name`
    Field(ValueAttributes, Box<Value>, Name),

    /// Field accessor function
    ///
    /// Example: `.name` as a function
    FieldFunction(ValueAttributes, Name),

    /// Function application
    ///
    /// Example: `f x` applies function `f` to argument `x`
    Apply(ValueAttributes, Box<Value>, Box<Value>),

    /// Lambda abstraction
    ///
    /// Example: `\x -> x + 1`
    Lambda(ValueAttributes, Pattern, Box<Value>),

    /// Let binding with a value definition
    ///
    /// Example: `let x = 1 in x + 1`
    LetDefinition(ValueAttributes, Name, Box<ValueDefinition>, Box<Value>),

    /// Recursive let bindings
    ///
    /// Example: `let rec f = ... and g = ... in ...`
    LetRecursion(ValueAttributes, Vec<LetBinding>, Box<Value>),

    /// Pattern destructuring in let
    ///
    /// Example: `let (a, b) = tuple in a + b`
    Destructure(ValueAttributes, Pattern, Box<Value>, Box<Value>),

    /// Conditional expression
    ///
    /// Example: `if cond then a else b`
    IfThenElse(ValueAttributes, Box<Value>, Box<Value>, Box<Value>),

    /// Pattern matching
    ///
    /// Example: `case x of Just v -> v; Nothing -> 0`
    PatternMatch(ValueAttributes, Box<Value>, Vec<PatternCase>),

    /// Record update
    ///
    /// Example: `{ person | name = "Bob" }`
    UpdateRecord(ValueAttributes, Box<Value>, Vec<RecordFieldEntry>),

    /// Unit value
    ///
    /// Example: `()`
    Unit(ValueAttributes),

    // === V4-only constructs ===
    /// Incomplete/broken value placeholder (V4 only)
    ///
    /// Represents values that couldn't be fully resolved or compiled.
    /// Used for incremental compilation and error recovery.
    Hole(ValueAttributes, HoleReason, Option<Box<Type>>),

    /// Native platform operation (V4 only)
    ///
    /// Represents operations that are implemented natively by the platform
    /// rather than having an IR body.
    Native(ValueAttributes, FQName, NativeInfo),

    /// External FFI call (V4 only)
    ///
    /// References an external function implementation.
    External(ValueAttributes, String, String), // external_name, target_platform
}

/// Reason why a value is incomplete/broken (V4 only)
#[derive(Debug, Clone, PartialEq)]
pub enum HoleReason {
    /// Reference couldn't be resolved
    UnresolvedReference { target: FQName },
    /// Value was removed during refactoring
    DeletedDuringRefactor,
    /// Type checking failed
    TypeMismatch,
    /// Work in progress, not yet implemented
    Draft,
}

/// Category hint for native operations (V4 only)
#[derive(Debug, Clone, PartialEq)]
pub enum NativeHint {
    Arithmetic,
    Comparison,
    StringOp,
    CollectionOp,
    PlatformSpecific,
}

/// Information about a native operation (V4 only)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeInfo {
    pub hint: NativeHint,
    pub description: Option<String>,
}

/// Input parameter tuple struct: (name, attributes, type)
///
/// More ergonomic than `(Name, ValueAttributes, Type)` - provides named fields via pattern matching.
#[derive(Debug, Clone, PartialEq)]
pub struct InputType(pub Name, pub ValueAttributes, pub Type);

/// Record field entry tuple struct: (name, value)
///
/// Used in Record and UpdateRecord value variants.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordFieldEntry(pub Name, pub Value);

/// Pattern match case tuple struct: (pattern, body)
#[derive(Debug, Clone, PartialEq)]
pub struct PatternCase(pub Pattern, pub Value);

/// Let-recursion binding tuple struct: (name, definition)
#[derive(Debug, Clone, PartialEq)]
pub struct LetBinding(pub Name, pub ValueDefinition);

/// The body of a value definition
///
/// V4 format supports Expression, Native, External, and Incomplete body types.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueBody {
    /// Normal expression body (all versions)
    Expression(Value),

    /// Native/builtin operation - no IR body (V4 only)
    Native(NativeInfo),

    /// External FFI definition (V4 only)
    External {
        external_name: String,
        target_platform: String,
    },

    /// Incomplete value definition (V4 only)
    Incomplete(HoleReason),
}

impl Value {
    /// Get the attributes of this value
    pub fn attributes(&self) -> &ValueAttributes {
        match self {
            Value::Literal(a, _) => a,
            Value::Constructor(a, _) => a,
            Value::Tuple(a, _) => a,
            Value::List(a, _) => a,
            Value::Record(a, _) => a,
            Value::Variable(a, _) => a,
            Value::Reference(a, _) => a,
            Value::Field(a, _, _) => a,
            Value::FieldFunction(a, _) => a,
            Value::Apply(a, _, _) => a,
            Value::Lambda(a, _, _) => a,
            Value::LetDefinition(a, _, _, _) => a,
            Value::LetRecursion(a, _, _) => a,
            Value::Destructure(a, _, _, _) => a,
            Value::IfThenElse(a, _, _, _) => a,
            Value::PatternMatch(a, _, _) => a,
            Value::UpdateRecord(a, _, _) => a,
            Value::Unit(a) => a,
            Value::Hole(a, _, _) => a,
            Value::Native(a, _, _) => a,
            Value::External(a, _, _) => a,
        }
    }

    /// Create a literal value
    pub fn literal(attrs: ValueAttributes, lit: Literal) -> Self {
        Value::Literal(attrs, lit)
    }

    /// Create a variable reference
    pub fn variable(attrs: ValueAttributes, name: Name) -> Self {
        Value::Variable(attrs, name)
    }

    /// Create a constructor reference
    pub fn constructor(attrs: ValueAttributes, name: FQName) -> Self {
        Value::Constructor(attrs, name)
    }

    /// Create a tuple
    pub fn tuple(attrs: ValueAttributes, elements: Vec<Value>) -> Self {
        Value::Tuple(attrs, elements)
    }

    /// Create a list
    pub fn list(attrs: ValueAttributes, elements: Vec<Value>) -> Self {
        Value::List(attrs, elements)
    }

    /// Create a record
    pub fn record(attrs: ValueAttributes, fields: Vec<RecordFieldEntry>) -> Self {
        Value::Record(attrs, fields)
    }

    /// Create a function application
    pub fn apply(attrs: ValueAttributes, function: Value, argument: Value) -> Self {
        Value::Apply(attrs, Box::new(function), Box::new(argument))
    }

    /// Create a lambda expression
    pub fn lambda(attrs: ValueAttributes, pattern: Pattern, body: Value) -> Self {
        Value::Lambda(attrs, pattern, Box::new(body))
    }

    /// Create an if-then-else expression
    pub fn if_then_else(
        attrs: ValueAttributes,
        condition: Value,
        then_branch: Value,
        else_branch: Value,
    ) -> Self {
        Value::IfThenElse(
            attrs,
            Box::new(condition),
            Box::new(then_branch),
            Box::new(else_branch),
        )
    }

    /// Create a unit value
    pub fn unit(attrs: ValueAttributes) -> Self {
        Value::Unit(attrs)
    }
}

// Convenience constructors for tuple structs
impl InputType {
    /// Create a new input type
    pub fn new(name: Name, attrs: ValueAttributes, tpe: Type) -> Self {
        InputType(name, attrs, tpe)
    }

    /// Get the name
    pub fn name(&self) -> &Name {
        &self.0
    }

    /// Get the attributes
    pub fn attrs(&self) -> &ValueAttributes {
        &self.1
    }

    /// Get the type
    pub fn tpe(&self) -> &Type {
        &self.2
    }
}

impl RecordFieldEntry {
    /// Create a new record field entry
    pub fn new(name: Name, value: Value) -> Self {
        RecordFieldEntry(name, value)
    }

    /// Get the name
    pub fn name(&self) -> &Name {
        &self.0
    }

    /// Get the value
    pub fn value(&self) -> &Value {
        &self.1
    }
}

impl PatternCase {
    /// Create a new pattern case
    pub fn new(pattern: Pattern, body: Value) -> Self {
        PatternCase(pattern, body)
    }

    /// Get the pattern
    pub fn pattern(&self) -> &Pattern {
        &self.0
    }

    /// Get the body
    pub fn body(&self) -> &Value {
        &self.1
    }
}

impl LetBinding {
    /// Create a new let binding
    pub fn new(name: Name, definition: ValueDefinition) -> Self {
        LetBinding(name, definition)
    }

    /// Get the name
    pub fn name(&self) -> &Name {
        &self.0
    }

    /// Get the definition
    pub fn definition(&self) -> &ValueDefinition {
        &self.1
    }
}

impl NativeInfo {
    /// Create a new NativeInfo
    pub fn new(hint: NativeHint, description: Option<String>) -> Self {
        NativeInfo { hint, description }
    }
}

// ============================================================================
// VALUE SPECIFICATIONS (Public API)
// ============================================================================

/// Value specification (just the signature)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueSpecification {
    pub inputs: IndexMap<String, Type>,
    pub output: Type,
}

// ============================================================================
// VALUE DEFINITIONS
// ============================================================================

/// A value definition (function or constant)
///
/// V4 format supports multiple body types (Expression, Native, External, Incomplete).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueDefinition {
    pub input_types: IndexMap<String, InputTypeEntry>,
    pub output_type: Type,
    pub body: ValueBody,
}

/// Input type entry
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputTypeEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_attributes: Option<ValueAttributes>,
    #[serde(rename = "type")]
    pub input_type: Type,
}

impl ValueDefinition {
    /// Create a new value definition with an expression body
    pub fn new(input_types: Vec<InputType>, output_type: Type, body: Value) -> Self {
        let inputs = input_types
            .into_iter()
            .map(|InputType(name, attrs, tpe)| {
                let entry = InputTypeEntry {
                    type_attributes: Some(attrs),
                    input_type: tpe,
                };
                (name.to_string(), entry)
            })
            .collect();

        ValueDefinition {
            input_types: inputs,
            output_type,
            body: ValueBody::Expression(body),
        }
    }

    /// Create a value definition with a native body (V4 only)
    pub fn native(input_types: Vec<InputType>, output_type: Type, info: NativeInfo) -> Self {
        let inputs = input_types
            .into_iter()
            .map(|InputType(name, attrs, tpe)| {
                let entry = InputTypeEntry {
                    type_attributes: Some(attrs),
                    input_type: tpe,
                };
                (name.to_string(), entry)
            })
            .collect();

        ValueDefinition {
            input_types: inputs,
            output_type,
            body: ValueBody::Native(info),
        }
    }
}

// ============================================================================
// SERIALIZATION SUPPORT FOR VALUE BODY
// ============================================================================

impl Serialize for ValueBody {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            ValueBody::Expression(body) => {
                map.serialize_entry("ExpressionBody", &ExpressionBodySerContent { body })?;
            }
            ValueBody::Native(info) => {
                map.serialize_entry(
                    "NativeBody",
                    &NativeBodySerContent {
                        hint: info.hint.clone(),
                        description: info.description.clone(),
                    },
                )?;
            }
            ValueBody::External {
                external_name,
                target_platform,
            } => {
                map.serialize_entry(
                    "ExternalBody",
                    &ExternalBodySerContent {
                        external_name: external_name.clone(),
                        target_platform: target_platform.clone(),
                    },
                )?;
            }
            ValueBody::Incomplete(reason) => {
                map.serialize_entry("IncompleteBody", &IncompleteBodySerContent { reason })?;
            }
        }
        map.end()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpressionBodySerContent<'a> {
    body: &'a Value,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeBodySerContent {
    hint: NativeHint,
    description: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExternalBodySerContent {
    external_name: String,
    target_platform: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IncompleteBodySerContent<'a> {
    reason: &'a HoleReason,
}

impl<'de> Deserialize<'de> for ValueBody {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if let serde_json::Value::Object(map) = &value {
            if let Some(content) = map.get("ExpressionBody") {
                let body_json = content
                    .get("body")
                    .ok_or_else(|| de::Error::missing_field("body"))?;
                let body: Value =
                    serde_json::from_value(body_json.clone()).map_err(de::Error::custom)?;
                return Ok(ValueBody::Expression(body));
            }
            if let Some(content) = map.get("NativeBody") {
                let parsed: NativeBodySerContent =
                    serde_json::from_value(content.clone()).map_err(de::Error::custom)?;
                return Ok(ValueBody::Native(NativeInfo {
                    hint: parsed.hint,
                    description: parsed.description,
                }));
            }
            if let Some(content) = map.get("ExternalBody") {
                let parsed: ExternalBodySerContent =
                    serde_json::from_value(content.clone()).map_err(de::Error::custom)?;
                return Ok(ValueBody::External {
                    external_name: parsed.external_name,
                    target_platform: parsed.target_platform,
                });
            }
            if let Some(content) = map.get("IncompleteBody") {
                let reason_val = content
                    .get("reason")
                    .ok_or_else(|| de::Error::missing_field("reason"))?;
                let reason: HoleReason =
                    serde_json::from_value(reason_val.clone()).map_err(de::Error::custom)?;
                return Ok(ValueBody::Incomplete(reason));
            }
        }
        Err(de::Error::custom(
            "expected ExpressionBody, NativeBody, ExternalBody, or IncompleteBody wrapper",
        ))
    }
}

// ============================================================================
// SERIALIZATION SUPPORT FOR NATIVE HINT
// ============================================================================

impl Serialize for NativeHint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            NativeHint::Arithmetic => map.serialize_entry("Arithmetic", &serde_json::json!({}))?,
            NativeHint::Comparison => map.serialize_entry("Comparison", &serde_json::json!({}))?,
            NativeHint::StringOp => map.serialize_entry("StringOp", &serde_json::json!({}))?,
            NativeHint::CollectionOp => {
                map.serialize_entry("CollectionOp", &serde_json::json!({}))?
            }
            NativeHint::PlatformSpecific => {
                map.serialize_entry("PlatformSpecific", &serde_json::json!({}))?
            }
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for NativeHint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match &value {
            serde_json::Value::Object(map) => {
                if let Some((key, _)) = map.iter().next() {
                    match key.as_str() {
                        "Arithmetic" => Ok(NativeHint::Arithmetic),
                        "Comparison" => Ok(NativeHint::Comparison),
                        "StringOp" => Ok(NativeHint::StringOp),
                        "CollectionOp" => Ok(NativeHint::CollectionOp),
                        "PlatformSpecific" => Ok(NativeHint::PlatformSpecific),
                        _ => Err(de::Error::unknown_variant(
                            key,
                            &[
                                "Arithmetic",
                                "Comparison",
                                "StringOp",
                                "CollectionOp",
                                "PlatformSpecific",
                            ],
                        )),
                    }
                } else {
                    Err(de::Error::custom("empty object for NativeHint"))
                }
            }
            // Also accept string format for backward compatibility
            serde_json::Value::String(s) => match s.as_str() {
                "Arithmetic" => Ok(NativeHint::Arithmetic),
                "Comparison" => Ok(NativeHint::Comparison),
                "StringOp" => Ok(NativeHint::StringOp),
                "CollectionOp" => Ok(NativeHint::CollectionOp),
                "PlatformSpecific" => Ok(NativeHint::PlatformSpecific),
                _ => Err(de::Error::unknown_variant(
                    s,
                    &[
                        "Arithmetic",
                        "Comparison",
                        "StringOp",
                        "CollectionOp",
                        "PlatformSpecific",
                    ],
                )),
            },
            _ => Err(de::Error::custom(
                "expected object or string for NativeHint",
            )),
        }
    }
}

// ============================================================================
// SERIALIZATION SUPPORT FOR HOLE REASON
// ============================================================================

impl Serialize for HoleReason {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            HoleReason::Draft => map.serialize_entry("Draft", &serde_json::json!({}))?,
            HoleReason::TypeMismatch => {
                map.serialize_entry("TypeMismatch", &serde_json::json!({}))?
            }
            HoleReason::DeletedDuringRefactor => {
                map.serialize_entry("DeletedDuringRefactor", &serde_json::json!({}))?
            }
            HoleReason::UnresolvedReference { target } => map.serialize_entry(
                "UnresolvedReference",
                &serde_json::json!({ "target": target.to_string() }),
            )?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for HoleReason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match &value {
            serde_json::Value::Object(map) => {
                if let Some((key, content)) = map.iter().next() {
                    match key.as_str() {
                        "Draft" => Ok(HoleReason::Draft),
                        "TypeMismatch" => Ok(HoleReason::TypeMismatch),
                        "DeletedDuringRefactor" => Ok(HoleReason::DeletedDuringRefactor),
                        "UnresolvedReference" => {
                            let target = content
                                .get("target")
                                .and_then(|t| t.as_str())
                                .ok_or_else(|| de::Error::missing_field("target"))?
                                .to_string();
                            Ok(HoleReason::UnresolvedReference {
                                target: FQName::from_canonical_string(&target)
                                    .map_err(|e| de::Error::custom(e))?,
                            })
                        }
                        _ => Err(de::Error::unknown_variant(
                            key,
                            &[
                                "Draft",
                                "TypeMismatch",
                                "DeletedDuringRefactor",
                                "UnresolvedReference",
                            ],
                        )),
                    }
                } else {
                    Err(de::Error::custom("empty object for HoleReason"))
                }
            }
            // Also accept string format for backward compatibility
            serde_json::Value::String(s) => match s.as_str() {
                "Draft" => Ok(HoleReason::Draft),
                "TypeMismatch" => Ok(HoleReason::TypeMismatch),
                "DeletedDuringRefactor" => Ok(HoleReason::DeletedDuringRefactor),
                _ => Err(de::Error::unknown_variant(
                    s,
                    &["Draft", "TypeMismatch", "DeletedDuringRefactor"],
                )),
            },
            _ => Err(de::Error::custom(
                "expected object or string for HoleReason",
            )),
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::super::attributes::TypeAttributes;
    use super::*;

    // Tests from value_expr.rs
    #[test]
    fn test_literal_value() {
        let val: Value = Value::literal(ValueAttributes::default(), Literal::Integer(42));
        assert!(matches!(val, Value::Literal(_, Literal::Integer(42))));
    }

    #[test]
    fn test_variable_value() {
        let val: Value = Value::variable(ValueAttributes::default(), Name::from("x"));
        assert!(matches!(val, Value::Variable(_, _)));
    }

    #[test]
    fn test_unit_value() {
        let val: Value = Value::unit(ValueAttributes::default());
        assert!(matches!(val, Value::Unit(_)));
    }

    #[test]
    fn test_tuple_value() {
        let val: Value = Value::tuple(
            ValueAttributes::default(),
            vec![
                Value::unit(ValueAttributes::default()),
                Value::unit(ValueAttributes::default()),
            ],
        );
        assert!(matches!(val, Value::Tuple(_, elements) if elements.len() == 2));
    }

    #[test]
    fn test_lambda_value() {
        let val: Value = Value::lambda(
            ValueAttributes::default(),
            Pattern::wildcard(ValueAttributes::default()),
            Value::unit(ValueAttributes::default()),
        );
        assert!(matches!(val, Value::Lambda(_, _, _)));
    }

    #[test]
    fn test_value_definition() {
        let def: ValueDefinition = ValueDefinition::new(
            vec![],
            Type::unit(TypeAttributes::default()),
            Value::unit(ValueAttributes::default()),
        );
        assert!(matches!(def.body, ValueBody::Expression(_)));
    }

    #[test]
    fn test_hole_value() {
        let val: Value = Value::Hole(ValueAttributes::default(), HoleReason::Draft, None);
        assert!(matches!(val, Value::Hole(_, HoleReason::Draft, None)));
    }

    #[test]
    fn test_native_value_definition() {
        let def: ValueDefinition = ValueDefinition::native(
            vec![],
            Type::unit(TypeAttributes::default()),
            NativeInfo::new(NativeHint::Arithmetic, Some("add operation".to_string())),
        );
        assert!(matches!(def.body, ValueBody::Native(_)));
    }

    // Tests from value_def.rs
    #[test]
    fn test_native_hint_wrapper_format() {
        let hint = NativeHint::Arithmetic;
        let json = serde_json::to_string(&hint).unwrap();
        assert!(json.contains("\"Arithmetic\""));
        assert!(json.contains("{}"));
    }

    #[test]
    fn test_hole_reason_wrapper_format() {
        let reason = HoleReason::Draft;
        let json = serde_json::to_string(&reason).unwrap();
        assert!(json.contains("\"Draft\""));
        assert!(json.contains("{}"));
    }

    #[test]
    fn test_hole_reason_with_target() {
        let reason = HoleReason::UnresolvedReference {
            target: FQName::from_canonical_string("my/pkg:mod#func").unwrap(),
        };
        let json = serde_json::to_string(&reason).unwrap();
        assert!(json.contains("\"UnresolvedReference\""));
        assert!(json.contains("\"target\""));
        // FQName serializes to canonical format my/pkg:mod:func
        assert!(json.contains("my/pkg:mod:func"));
    }

    #[test]
    fn test_value_body_expression_wrapper() {
        let body = ValueBody::Expression(Value::Unit(ValueAttributes::default()));
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"ExpressionBody\""));
        assert!(json.contains("\"body\""));
    }
}

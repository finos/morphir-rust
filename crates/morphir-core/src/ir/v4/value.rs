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
use super::types::{Incompleteness, Type};
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
    DeletedDuringRefactor {
        /// Transaction ID of the refactoring that deleted this reference
        tx_id: String,
    },
    /// Type checking failed
    TypeMismatch {
        /// Expected type description
        expected: String,
        /// Actual type found
        found: String,
    },
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
    PlatformSpecific {
        /// Platform identifier (e.g., "wasm", "javascript", "native")
        platform: String,
    },
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
#[allow(clippy::large_enum_variant)]
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
    Incomplete {
        incompleteness: Incompleteness,
        partial_body: Option<Value>,
    },
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
#[derive(Debug, Clone, PartialEq)]
pub struct ValueDefinition {
    pub input_types: IndexMap<String, InputTypeEntry>,
    pub output_type: Option<Type>,
    pub body: ValueBody,
}

fn serialize_input_types<S>(
    input_types: &IndexMap<String, InputTypeEntry>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    input_types
        .iter()
        .map(|(name, entry)| (name, &entry.input_type))
        .collect::<IndexMap<_, _>>()
        .serialize(serializer)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpressionDefinitionContent<'a> {
    #[serde(serialize_with = "serialize_input_types")]
    input_types: &'a IndexMap<String, InputTypeEntry>,
    output_type: &'a Type,
    body: &'a Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeDefinitionContent<'a> {
    #[serde(serialize_with = "serialize_input_types")]
    input_types: &'a IndexMap<String, InputTypeEntry>,
    output_type: &'a Type,
    native_info: &'a NativeInfo,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalDefinitionContent<'a> {
    #[serde(serialize_with = "serialize_input_types")]
    input_types: &'a IndexMap<String, InputTypeEntry>,
    output_type: &'a Type,
    external_name: &'a str,
    target_platform: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IncompleteDefinitionContent<'a> {
    #[serde(serialize_with = "serialize_input_types")]
    input_types: &'a IndexMap<String, InputTypeEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_type: &'a Option<Type>,
    incompleteness: &'a Incompleteness,
    #[serde(skip_serializing_if = "Option::is_none")]
    partial_body: &'a Option<Value>,
}

impl Serialize for ValueDefinition {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        match &self.body {
            ValueBody::Expression(body) => map.serialize_entry(
                "ExpressionBody",
                &ExpressionDefinitionContent {
                    input_types: &self.input_types,
                    output_type: self.output_type.as_ref().ok_or_else(|| {
                        serde::ser::Error::custom("ExpressionBody requires outputType")
                    })?,
                    body,
                },
            )?,
            ValueBody::Native(native_info) => map.serialize_entry(
                "NativeBody",
                &NativeDefinitionContent {
                    input_types: &self.input_types,
                    output_type: self.output_type.as_ref().ok_or_else(|| {
                        serde::ser::Error::custom("NativeBody requires outputType")
                    })?,
                    native_info,
                },
            )?,
            ValueBody::External {
                external_name,
                target_platform,
            } => map.serialize_entry(
                "ExternalBody",
                &ExternalDefinitionContent {
                    input_types: &self.input_types,
                    output_type: self.output_type.as_ref().ok_or_else(|| {
                        serde::ser::Error::custom("ExternalBody requires outputType")
                    })?,
                    external_name,
                    target_platform,
                },
            )?,
            ValueBody::Incomplete {
                incompleteness,
                partial_body,
            } => map.serialize_entry(
                "IncompleteBody",
                &IncompleteDefinitionContent {
                    input_types: &self.input_types,
                    output_type: &self.output_type,
                    incompleteness,
                    partial_body,
                },
            )?,
        }
        map.end()
    }
}

fn deserialize_input_types<E: de::Error>(
    values: IndexMap<String, serde_json::Value>,
) -> Result<IndexMap<String, InputTypeEntry>, E> {
    values
        .into_iter()
        .map(|(name, value)| {
            let entry = serde_json::from_value::<Type>(value.clone())
                .map(|input_type| InputTypeEntry {
                    type_attributes: None,
                    input_type,
                })
                .or_else(|_| serde_json::from_value::<InputTypeEntry>(value))
                .map_err(de::Error::custom)?;
            Ok((name, entry))
        })
        .collect()
}

impl<'de> Deserialize<'de> for ValueDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Common {
            #[serde(default)]
            input_types: IndexMap<String, serde_json::Value>,
            output_type: Option<Type>,
            body: Option<Value>,
            native_info: Option<NativeInfo>,
            external_name: Option<String>,
            target_platform: Option<String>,
            incompleteness: Option<Incompleteness>,
            partial_body: Option<Value>,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| de::Error::custom("expected a value definition wrapper"))?;
        if object.contains_key("inputTypes") {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Legacy {
                #[serde(default)]
                input_types: IndexMap<String, serde_json::Value>,
                output_type: Option<Type>,
                body: ValueBody,
            }
            let legacy: Legacy = serde_json::from_value(value).map_err(de::Error::custom)?;
            return Ok(Self {
                input_types: deserialize_input_types(legacy.input_types)?,
                output_type: legacy.output_type,
                body: legacy.body,
            });
        }
        let (tag, content) = object
            .iter()
            .next()
            .ok_or_else(|| de::Error::custom("empty value definition wrapper"))?;
        let content: Common = serde_json::from_value(content.clone()).map_err(de::Error::custom)?;
        let input_types = deserialize_input_types(content.input_types)?;
        let body = match tag.as_str() {
            "ExpressionBody" => ValueBody::Expression(
                content
                    .body
                    .ok_or_else(|| de::Error::missing_field("body"))?,
            ),
            "NativeBody" => ValueBody::Native(
                content
                    .native_info
                    .ok_or_else(|| de::Error::missing_field("nativeInfo"))?,
            ),
            "ExternalBody" => ValueBody::External {
                external_name: content
                    .external_name
                    .ok_or_else(|| de::Error::missing_field("externalName"))?,
                target_platform: content
                    .target_platform
                    .ok_or_else(|| de::Error::missing_field("targetPlatform"))?,
            },
            "IncompleteBody" => ValueBody::Incomplete {
                incompleteness: content
                    .incompleteness
                    .ok_or_else(|| de::Error::missing_field("incompleteness"))?,
                partial_body: content.partial_body,
            },
            _ => {
                return Err(de::Error::unknown_variant(
                    tag,
                    &[
                        "ExpressionBody",
                        "NativeBody",
                        "ExternalBody",
                        "IncompleteBody",
                    ],
                ));
            }
        };
        Ok(Self {
            input_types,
            output_type: content.output_type,
            body,
        })
    }
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
            output_type: Some(output_type),
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
            output_type: Some(output_type),
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
            ValueBody::Incomplete {
                incompleteness,
                partial_body,
            } => {
                map.serialize_entry(
                    "IncompleteBody",
                    &IncompleteBodySerContent {
                        incompleteness,
                        partial_body: partial_body.as_ref(),
                    },
                )?;
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
    incompleteness: &'a Incompleteness,
    #[serde(skip_serializing_if = "Option::is_none")]
    partial_body: Option<&'a Value>,
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
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct IncompleteBodyContent {
                    incompleteness: Incompleteness,
                    #[serde(default)]
                    partial_body: Option<Value>,
                }

                let parsed: IncompleteBodyContent =
                    serde_json::from_value(content.clone()).map_err(de::Error::custom)?;
                return Ok(ValueBody::Incomplete {
                    incompleteness: parsed.incompleteness,
                    partial_body: parsed.partial_body,
                });
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
            NativeHint::PlatformSpecific { platform } => map.serialize_entry(
                "PlatformSpecific",
                &serde_json::json!({ "platform": platform }),
            )?,
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
                if let Some((key, content)) = map.iter().next() {
                    match key.as_str() {
                        "Arithmetic" => Ok(NativeHint::Arithmetic),
                        "Comparison" => Ok(NativeHint::Comparison),
                        "StringOp" => Ok(NativeHint::StringOp),
                        "CollectionOp" => Ok(NativeHint::CollectionOp),
                        "PlatformSpecific" => {
                            let platform = content
                                .get("platform")
                                .and_then(|p| p.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            Ok(NativeHint::PlatformSpecific { platform })
                        }
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
                "PlatformSpecific" => Ok(NativeHint::PlatformSpecific {
                    platform: "unknown".to_string(),
                }),
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
            HoleReason::TypeMismatch { expected, found } => map.serialize_entry(
                "TypeMismatch",
                &serde_json::json!({ "expected": expected, "found": found }),
            )?,
            HoleReason::DeletedDuringRefactor { tx_id } => map.serialize_entry(
                "DeletedDuringRefactor",
                &serde_json::json!({ "tx-id": tx_id }),
            )?,
            HoleReason::UnresolvedReference { target } => map.serialize_entry(
                "UnresolvedReference",
                &serde_json::json!({ "target": target.to_canonical_string() }),
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
                        "TypeMismatch" => {
                            let expected = content
                                .get("expected")
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| de::Error::missing_field("expected"))?
                                .to_string();
                            let found = content
                                .get("found")
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| de::Error::missing_field("found"))?
                                .to_string();
                            Ok(HoleReason::TypeMismatch { expected, found })
                        }
                        "DeletedDuringRefactor" => {
                            let tx_id = content
                                .get("tx-id")
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| de::Error::missing_field("tx-id"))?
                                .to_string();
                            Ok(HoleReason::DeletedDuringRefactor { tx_id })
                        }
                        "UnresolvedReference" => {
                            let target = content
                                .get("target")
                                .and_then(|t| t.as_str())
                                .ok_or_else(|| de::Error::missing_field("target"))?
                                .to_string();
                            Ok(HoleReason::UnresolvedReference {
                                target: FQName::from_canonical_string(&target)
                                    .map_err(de::Error::custom)?,
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
            // Also accept string format for backward compatibility (Draft only)
            serde_json::Value::String(s) => match s.as_str() {
                "Draft" => Ok(HoleReason::Draft),
                _ => Err(de::Error::unknown_variant(s, &["Draft"])),
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
        let val: Value = Value::Hole(
            ValueAttributes::default(),
            HoleReason::TypeMismatch {
                expected: "Int".to_string(),
                found: "String".to_string(),
            },
            None,
        );
        assert!(matches!(
            val,
            Value::Hole(_, HoleReason::TypeMismatch { .. }, None)
        ));
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
        // FQName serializes to canonical format with # separator
        assert!(json.contains("my/pkg:mod#func"));
    }

    #[test]
    fn test_value_body_expression_wrapper() {
        let body = ValueBody::Expression(Value::Unit(ValueAttributes::default()));
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"ExpressionBody\""));
        assert!(json.contains("\"body\""));
    }
}

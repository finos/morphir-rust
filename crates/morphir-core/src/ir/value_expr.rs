//! Value expressions for Morphir IR.
//!
//! This module defines the `Value` enum which represents value expressions
//! in the Morphir IR. Values use `TypeAttributes` for type nodes and
//! `ValueAttributes` for value nodes (V4 format).
//!
//! # Examples
//!
//! ```rust,ignore
//! let v: Value = Value::Unit(ValueAttributes::default());
//! ```

use super::attributes::ValueAttributes;
use super::literal::Literal;
use super::pattern::Pattern;
use super::type_expr::Type;
use crate::naming::{FQName, Name};

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
///
/// Note: Draft is handled separately in Incompleteness, not here.
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
        /// Actual type description
        found: String,
    },
}

/// Category hint for native operations (V4 only)
///
/// Categorization hint for native operations used by code generators.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeHint {
    /// Basic arithmetic/logic operation
    Arithmetic,
    /// Comparison operation
    Comparison,
    /// String operation
    StringOp,
    /// Collection operation
    CollectionOp,
    /// Platform-specific operation
    PlatformSpecific {
        /// Platform identifier (e.g., 'wasm', 'javascript', 'native')
        platform: String,
    },
}

/// Information about a native operation (V4 only)
#[derive(Debug, Clone, PartialEq)]
pub struct NativeInfo {
    pub hint: NativeHint,
    pub description: Option<String>,
}

/// A value definition (function or constant)
///
/// V4 format supports multiple body types (Expression, Native, External, Incomplete).
#[derive(Debug, Clone, PartialEq)]
pub struct ValueDefinition {
    /// Input parameters with their names, attributes, and types
    pub input_types: Vec<InputType>,
    /// Output/return type
    pub output_type: Type,
    /// The body of the definition
    pub body: ValueBody,
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

impl ValueDefinition {
    /// Create a new value definition with an expression body
    pub fn new(input_types: Vec<InputType>, output_type: Type, body: Value) -> Self {
        ValueDefinition {
            input_types,
            output_type,
            body: ValueBody::Expression(body),
        }
    }

    /// Create a value definition with a native body (V4 only)
    pub fn native(input_types: Vec<InputType>, output_type: Type, info: NativeInfo) -> Self {
        ValueDefinition {
            input_types,
            output_type,
            body: ValueBody::Native(info),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::attributes::TypeAttributes;

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
}

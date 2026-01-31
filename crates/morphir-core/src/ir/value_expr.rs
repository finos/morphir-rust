//! Value expressions for Morphir IR.
//!
//! This module defines the `Value<TA, VA>` enum which represents value expressions
//! in the Morphir IR. The type parameters represent:
//! - `TA`: Type attributes (attached to type nodes)
//! - `VA`: Value attributes (attached to value nodes)
//!
//! # Default Type Parameters
//!
//! V4 is the default format - `Value` without type parameters uses V4 attributes:
//! ```rust,ignore
//! let v: Value = Value::Unit(ValueAttributes::default());  // V4 value
//! let v: Value<ClassicAttrs, ClassicAttrs> = Value::Unit(json!({}));  // Classic
//! ```

use super::attributes::{TypeAttributes, ValueAttributes};
use super::literal::Literal;
use super::pattern::Pattern;
use super::type_expr::Type;
use crate::naming::{FQName, Name};

/// A value expression with generic type and value attributes.
///
/// Value expressions form the term-level representation in Morphir IR.
/// Each variant carries value attributes of type `VA`, and types within
/// carry type attributes of type `TA`.
///
/// # Type Parameters
/// - `TA`: The type of attributes attached to type nodes.
///   Defaults to `TypeAttributes` (V4 format).
/// - `VA`: The type of attributes attached to value nodes.
///   Defaults to `ValueAttributes` (V4 format).
///
/// # Examples
///
/// ```rust,ignore
/// // V4 format (default)
/// let v: Value = Value::Unit(ValueAttributes::default());
///
/// // Classic format - explicit attributes
/// let v: Value<serde_json::Value, serde_json::Value> = Value::Unit(serde_json::json!({}));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum Value<TA: Clone = TypeAttributes, VA: Clone = ValueAttributes> {
    // === Core expressions (all versions) ===
    /// Literal constant value
    ///
    /// Example: `42`, `"hello"`, `true`
    Literal(VA, Literal),

    /// Data constructor reference
    ///
    /// Example: `Just` in `Just 42`
    Constructor(VA, FQName),

    /// Tuple construction
    ///
    /// Example: `(1, "hello", true)`
    Tuple(VA, Vec<Value<TA, VA>>),

    /// List construction
    ///
    /// Example: `[1, 2, 3]`
    List(VA, Vec<Value<TA, VA>>),

    /// Record construction
    ///
    /// Example: `{ name = "Alice", age = 30 }`
    Record(VA, Vec<RecordFieldEntry<TA, VA>>),

    /// Variable reference
    ///
    /// Example: `x` in `let x = 1 in x + 1`
    Variable(VA, Name),

    /// Reference to a named value
    ///
    /// Example: `List.map` referencing a module function
    Reference(VA, FQName),

    /// Field access on a record
    ///
    /// Example: `person.name`
    Field(VA, Box<Value<TA, VA>>, Name),

    /// Field accessor function
    ///
    /// Example: `.name` as a function
    FieldFunction(VA, Name),

    /// Function application
    ///
    /// Example: `f x` applies function `f` to argument `x`
    Apply(VA, Box<Value<TA, VA>>, Box<Value<TA, VA>>),

    /// Lambda abstraction
    ///
    /// Example: `\x -> x + 1`
    Lambda(VA, Pattern<VA>, Box<Value<TA, VA>>),

    /// Let binding with a value definition
    ///
    /// Example: `let x = 1 in x + 1`
    LetDefinition(VA, Name, Box<ValueDefinition<TA, VA>>, Box<Value<TA, VA>>),

    /// Recursive let bindings
    ///
    /// Example: `let rec f = ... and g = ... in ...`
    LetRecursion(VA, Vec<LetBinding<TA, VA>>, Box<Value<TA, VA>>),

    /// Pattern destructuring in let
    ///
    /// Example: `let (a, b) = tuple in a + b`
    Destructure(VA, Pattern<VA>, Box<Value<TA, VA>>, Box<Value<TA, VA>>),

    /// Conditional expression
    ///
    /// Example: `if cond then a else b`
    IfThenElse(
        VA,
        Box<Value<TA, VA>>,
        Box<Value<TA, VA>>,
        Box<Value<TA, VA>>,
    ),

    /// Pattern matching
    ///
    /// Example: `case x of Just v -> v; Nothing -> 0`
    PatternMatch(VA, Box<Value<TA, VA>>, Vec<PatternCase<TA, VA>>),

    /// Record update
    ///
    /// Example: `{ person | name = "Bob" }`
    UpdateRecord(VA, Box<Value<TA, VA>>, Vec<RecordFieldEntry<TA, VA>>),

    /// Unit value
    ///
    /// Example: `()`
    Unit(VA),

    // === V4-only constructs ===
    /// Incomplete/broken value placeholder (V4 only)
    ///
    /// Represents values that couldn't be fully resolved or compiled.
    /// Used for incremental compilation and error recovery.
    Hole(VA, HoleReason, Option<Box<Type<TA>>>),

    /// Native platform operation (V4 only)
    ///
    /// Represents operations that are implemented natively by the platform
    /// rather than having an IR body.
    Native(VA, FQName, NativeInfo),

    /// External FFI call (V4 only)
    ///
    /// References an external function implementation.
    External(VA, String, String), // external_name, target_platform
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
/// Classic format always has an expression body.
/// V4 format supports additional body types (Native, External, Incomplete).
///
/// # Type Parameters
/// - `TA`: Type attributes. Defaults to `TypeAttributes` (V4).
/// - `VA`: Value attributes. Defaults to `ValueAttributes` (V4).
#[derive(Debug, Clone, PartialEq)]
pub struct ValueDefinition<TA: Clone = TypeAttributes, VA: Clone = ValueAttributes> {
    /// Input parameters with their names, attributes, and types
    pub input_types: Vec<InputType<TA, VA>>,
    /// Output/return type
    pub output_type: Type<TA>,
    /// The body of the definition
    pub body: ValueBody<TA, VA>,
}

/// Input parameter tuple struct: (name, attributes, type)
///
/// More ergonomic than `(Name, VA, Type<TA>)` - provides named fields via pattern matching.
#[derive(Debug, Clone, PartialEq)]
pub struct InputType<TA: Clone = TypeAttributes, VA: Clone = ValueAttributes>(
    pub Name,
    pub VA,
    pub Type<TA>,
);

/// Record field entry tuple struct: (name, value)
///
/// Used in Record and UpdateRecord value variants.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordFieldEntry<TA: Clone = TypeAttributes, VA: Clone = ValueAttributes>(
    pub Name,
    pub Value<TA, VA>,
);

/// Pattern match case tuple struct: (pattern, body)
#[derive(Debug, Clone, PartialEq)]
pub struct PatternCase<TA: Clone = TypeAttributes, VA: Clone = ValueAttributes>(
    pub Pattern<VA>,
    pub Value<TA, VA>,
);

/// Let-recursion binding tuple struct: (name, definition)
#[derive(Debug, Clone, PartialEq)]
pub struct LetBinding<TA: Clone = TypeAttributes, VA: Clone = ValueAttributes>(
    pub Name,
    pub ValueDefinition<TA, VA>,
);

/// The body of a value definition
///
/// Classic format only supports Expression bodies.
/// V4 format adds Native, External, and Incomplete body types.
///
/// # Type Parameters
/// - `TA`: Type attributes. Defaults to `TypeAttributes` (V4).
/// - `VA`: Value attributes. Defaults to `ValueAttributes` (V4).
#[derive(Debug, Clone, PartialEq)]
pub enum ValueBody<TA: Clone = TypeAttributes, VA: Clone = ValueAttributes> {
    /// Normal expression body (all versions)
    Expression(Value<TA, VA>),

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

impl<TA: Clone, VA: Clone> Value<TA, VA> {
    /// Get the attributes of this value
    pub fn attributes(&self) -> &VA {
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
    pub fn literal(attrs: VA, lit: Literal) -> Self {
        Value::Literal(attrs, lit)
    }

    /// Create a variable reference
    pub fn variable(attrs: VA, name: Name) -> Self {
        Value::Variable(attrs, name)
    }

    /// Create a constructor reference
    pub fn constructor(attrs: VA, name: FQName) -> Self {
        Value::Constructor(attrs, name)
    }

    /// Create a tuple
    pub fn tuple(attrs: VA, elements: Vec<Value<TA, VA>>) -> Self {
        Value::Tuple(attrs, elements)
    }

    /// Create a list
    pub fn list(attrs: VA, elements: Vec<Value<TA, VA>>) -> Self {
        Value::List(attrs, elements)
    }

    /// Create a record
    pub fn record(attrs: VA, fields: Vec<RecordFieldEntry<TA, VA>>) -> Self {
        Value::Record(attrs, fields)
    }

    /// Create a function application
    pub fn apply(attrs: VA, function: Value<TA, VA>, argument: Value<TA, VA>) -> Self {
        Value::Apply(attrs, Box::new(function), Box::new(argument))
    }

    /// Create a lambda expression
    pub fn lambda(attrs: VA, pattern: Pattern<VA>, body: Value<TA, VA>) -> Self {
        Value::Lambda(attrs, pattern, Box::new(body))
    }

    /// Create an if-then-else expression
    pub fn if_then_else(
        attrs: VA,
        condition: Value<TA, VA>,
        then_branch: Value<TA, VA>,
        else_branch: Value<TA, VA>,
    ) -> Self {
        Value::IfThenElse(
            attrs,
            Box::new(condition),
            Box::new(then_branch),
            Box::new(else_branch),
        )
    }

    /// Create a unit value
    pub fn unit(attrs: VA) -> Self {
        Value::Unit(attrs)
    }
}

impl<TA: Clone, VA: Clone> ValueDefinition<TA, VA> {
    /// Create a new value definition with an expression body
    pub fn new(
        input_types: Vec<InputType<TA, VA>>,
        output_type: Type<TA>,
        body: Value<TA, VA>,
    ) -> Self {
        ValueDefinition {
            input_types,
            output_type,
            body: ValueBody::Expression(body),
        }
    }

    /// Create a value definition with a native body (V4 only)
    pub fn native(
        input_types: Vec<InputType<TA, VA>>,
        output_type: Type<TA>,
        info: NativeInfo,
    ) -> Self {
        ValueDefinition {
            input_types,
            output_type,
            body: ValueBody::Native(info),
        }
    }
}

// Convenience constructors for tuple structs
impl<TA: Clone, VA: Clone> InputType<TA, VA> {
    /// Create a new input type
    pub fn new(name: Name, attrs: VA, tpe: Type<TA>) -> Self {
        InputType(name, attrs, tpe)
    }

    /// Get the name
    pub fn name(&self) -> &Name {
        &self.0
    }

    /// Get the attributes
    pub fn attrs(&self) -> &VA {
        &self.1
    }

    /// Get the type
    pub fn tpe(&self) -> &Type<TA> {
        &self.2
    }
}

impl<TA: Clone, VA: Clone> RecordFieldEntry<TA, VA> {
    /// Create a new record field entry
    pub fn new(name: Name, value: Value<TA, VA>) -> Self {
        RecordFieldEntry(name, value)
    }

    /// Get the name
    pub fn name(&self) -> &Name {
        &self.0
    }

    /// Get the value
    pub fn value(&self) -> &Value<TA, VA> {
        &self.1
    }
}

impl<TA: Clone, VA: Clone> PatternCase<TA, VA> {
    /// Create a new pattern case
    pub fn new(pattern: Pattern<VA>, body: Value<TA, VA>) -> Self {
        PatternCase(pattern, body)
    }

    /// Get the pattern
    pub fn pattern(&self) -> &Pattern<VA> {
        &self.0
    }

    /// Get the body
    pub fn body(&self) -> &Value<TA, VA> {
        &self.1
    }
}

impl<TA: Clone, VA: Clone> LetBinding<TA, VA> {
    /// Create a new let binding
    pub fn new(name: Name, definition: ValueDefinition<TA, VA>) -> Self {
        LetBinding(name, definition)
    }

    /// Get the name
    pub fn name(&self) -> &Name {
        &self.0
    }

    /// Get the definition
    pub fn definition(&self) -> &ValueDefinition<TA, VA> {
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

    #[test]
    fn test_literal_value() {
        let val: Value<(), ()> = Value::literal((), Literal::Integer(42));
        assert!(matches!(val, Value::Literal(_, Literal::Integer(42))));
    }

    #[test]
    fn test_variable_value() {
        let val: Value<(), ()> = Value::variable((), Name::from("x"));
        assert!(matches!(val, Value::Variable(_, _)));
    }

    #[test]
    fn test_unit_value() {
        let val: Value<(), ()> = Value::unit(());
        assert!(matches!(val, Value::Unit(_)));
    }

    #[test]
    fn test_tuple_value() {
        let val: Value<(), ()> = Value::tuple((), vec![Value::unit(()), Value::unit(())]);
        assert!(matches!(val, Value::Tuple(_, elements) if elements.len() == 2));
    }

    #[test]
    fn test_lambda_value() {
        let val: Value<(), ()> = Value::lambda((), Pattern::wildcard(()), Value::unit(()));
        assert!(matches!(val, Value::Lambda(_, _, _)));
    }

    #[test]
    fn test_value_definition() {
        let def: ValueDefinition<(), ()> =
            ValueDefinition::new(vec![], Type::unit(()), Value::unit(()));
        assert!(matches!(def.body, ValueBody::Expression(_)));
    }

    #[test]
    fn test_hole_value() {
        let val: Value<(), ()> = Value::Hole(
            (),
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
        let def: ValueDefinition<(), ()> = ValueDefinition::native(
            vec![],
            Type::unit(()),
            NativeInfo::new(NativeHint::Arithmetic, Some("add operation".to_string())),
        );
        assert!(matches!(def.body, ValueBody::Native(_)));
    }
}

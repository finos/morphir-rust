//! Value expressions for Morphir IR.
//!
//! This module defines the `Value<TA, VA>` enum which represents value expressions
//! in the Morphir IR. The type parameters represent:
//! - `TA`: Type attributes (attached to type nodes)
//! - `VA`: Value attributes (attached to value nodes)

use crate::naming::{FQName, Name};
use super::literal::Literal;
use super::pattern::Pattern;
use super::type_expr::Type;

/// A value expression with generic type and value attributes.
///
/// Value expressions form the term-level representation in Morphir IR.
/// Each variant carries value attributes of type `VA`, and types within
/// carry type attributes of type `TA`.
///
/// # Type Parameters
/// - `TA`: The type of attributes attached to type nodes
/// - `VA`: The type of attributes attached to value nodes
#[derive(Debug, Clone, PartialEq)]
pub enum Value<TA, VA> {
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
    Record(VA, Vec<(Name, Value<TA, VA>)>),

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
    LetRecursion(VA, Vec<(Name, ValueDefinition<TA, VA>)>, Box<Value<TA, VA>>),

    /// Pattern destructuring in let
    ///
    /// Example: `let (a, b) = tuple in a + b`
    Destructure(VA, Pattern<VA>, Box<Value<TA, VA>>, Box<Value<TA, VA>>),

    /// Conditional expression
    ///
    /// Example: `if cond then a else b`
    IfThenElse(VA, Box<Value<TA, VA>>, Box<Value<TA, VA>>, Box<Value<TA, VA>>),

    /// Pattern matching
    ///
    /// Example: `case x of Just v -> v; Nothing -> 0`
    PatternMatch(VA, Box<Value<TA, VA>>, Vec<(Pattern<VA>, Value<TA, VA>)>),

    /// Record update
    ///
    /// Example: `{ person | name = "Bob" }`
    UpdateRecord(VA, Box<Value<TA, VA>>, Vec<(Name, Value<TA, VA>)>),

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
#[derive(Debug, Clone, PartialEq)]
pub struct NativeInfo {
    pub hint: NativeHint,
    pub description: Option<String>,
}

/// A value definition (function or constant)
///
/// Classic format always has an expression body.
/// V4 format supports additional body types (Native, External, Incomplete).
#[derive(Debug, Clone, PartialEq)]
pub struct ValueDefinition<TA, VA> {
    /// Input parameters with their names, attributes, and types
    pub input_types: Vec<(Name, VA, Type<TA>)>,
    /// Output/return type
    pub output_type: Type<TA>,
    /// The body of the definition
    pub body: ValueBody<TA, VA>,
}

/// The body of a value definition
///
/// Classic format only supports Expression bodies.
/// V4 format adds Native, External, and Incomplete body types.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueBody<TA, VA> {
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

impl<TA, VA> Value<TA, VA> {
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
    pub fn record(attrs: VA, fields: Vec<(Name, Value<TA, VA>)>) -> Self {
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

impl<TA, VA> ValueDefinition<TA, VA> {
    /// Create a new value definition with an expression body
    pub fn new(
        input_types: Vec<(Name, VA, Type<TA>)>,
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
        input_types: Vec<(Name, VA, Type<TA>)>,
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

impl NativeInfo {
    /// Create a new NativeInfo
    pub fn new(hint: NativeHint, description: Option<String>) -> Self {
        NativeInfo { hint, description }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_attrs() -> () {
        ()
    }

    #[test]
    fn test_literal_value() {
        let val: Value<(), ()> = Value::literal(test_attrs(), Literal::Integer(42));
        assert!(matches!(val, Value::Literal(_, Literal::Integer(42))));
    }

    #[test]
    fn test_variable_value() {
        let val: Value<(), ()> = Value::variable(test_attrs(), Name::from("x"));
        assert!(matches!(val, Value::Variable(_, _)));
    }

    #[test]
    fn test_unit_value() {
        let val: Value<(), ()> = Value::unit(test_attrs());
        assert!(matches!(val, Value::Unit(_)));
    }

    #[test]
    fn test_tuple_value() {
        let val: Value<(), ()> = Value::tuple(
            test_attrs(),
            vec![Value::unit(test_attrs()), Value::unit(test_attrs())],
        );
        assert!(matches!(val, Value::Tuple(_, elements) if elements.len() == 2));
    }

    #[test]
    fn test_lambda_value() {
        let val: Value<(), ()> = Value::lambda(
            test_attrs(),
            Pattern::wildcard(test_attrs()),
            Value::unit(test_attrs()),
        );
        assert!(matches!(val, Value::Lambda(_, _, _)));
    }

    #[test]
    fn test_value_definition() {
        let def: ValueDefinition<(), ()> = ValueDefinition::new(
            vec![],
            Type::unit(test_attrs()),
            Value::unit(test_attrs()),
        );
        assert!(matches!(def.body, ValueBody::Expression(_)));
    }

    #[test]
    fn test_hole_value() {
        let val: Value<(), ()> = Value::Hole(
            test_attrs(),
            HoleReason::Draft,
            None,
        );
        assert!(matches!(val, Value::Hole(_, HoleReason::Draft, None)));
    }

    #[test]
    fn test_native_value_definition() {
        let def: ValueDefinition<(), ()> = ValueDefinition::native(
            vec![],
            Type::unit(test_attrs()),
            NativeInfo::new(NativeHint::Arithmetic, Some("add operation".to_string())),
        );
        assert!(matches!(def.body, ValueBody::Native(_)));
    }
}

//! Morphir IR (Intermediate Representation) data structures
//!
//! This module defines the core data structures for Morphir IR following
//! functional domain modeling principles with immutable, type-safe structures.

use serde::{Deserialize, Serialize};

/// Morphir IR package structure
///
/// A package is the top-level unit of organization in Morphir IR.
///
/// Note: This type implements `PartialEq` but not `Eq` because it contains types
/// with `Expression` which cannot implement `Eq` due to `f64` values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Package {
    /// Package name
    pub name: String,
    /// Modules contained in this package
    pub modules: Vec<Module>,
}

/// Morphir IR module structure
///
/// A module contains type definitions, value definitions, and other declarations.
///
/// Note: This type implements `PartialEq` but not `Eq` because it contains types
/// with `Expression` which cannot implement `Eq` due to `f64` values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Module {
    /// Module name
    pub name: String,
    /// Type definitions in this module
    pub types: Vec<TypeDefinition>,
    /// Value definitions in this module
    pub values: Vec<ValueDefinition>,
}

/// Type definition in a module
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeDefinition {
    /// Type name
    pub name: String,
    /// Type expression
    pub typ: TypeExpression,
}

/// Value definition in a module
///
/// Note: This type implements `PartialEq` but not `Eq` because it contains `Expression`
/// which cannot implement `Eq` due to `f64` values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueDefinition {
    /// Value name
    pub name: String,
    /// Value type
    pub typ: TypeExpression,
    /// Value expression
    pub expr: Expression,
}

/// Type expression in Morphir IR
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TypeExpression {
    /// Variable type
    Variable { name: String },
    /// Unit type
    Unit,
    /// Function type
    Function {
        parameter: Box<TypeExpression>,
        return_type: Box<TypeExpression>,
    },
    /// Record type
    Record { fields: Vec<Field> },
    /// Tuple type
    Tuple { elements: Vec<TypeExpression> },
    /// Custom type reference
    Reference { name: String, parameters: Vec<TypeExpression> },
}

/// Field in a record type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// Field name
    pub name: String,
    /// Field type
    pub typ: TypeExpression,
}

/// Expression in Morphir IR
///
/// Note: This type implements `PartialEq` but not `Eq` because it contains `Literal`
/// which cannot implement `Eq` due to `f64` values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Expression {
    /// Literal value
    Literal { value: Literal },
    /// Variable reference
    Variable { name: String },
    /// Function application
    Apply {
        function: Box<Expression>,
        argument: Box<Expression>,
    },
    /// Lambda abstraction
    Lambda {
        parameter: String,
        body: Box<Expression>,
    },
    /// Let binding
    Let {
        bindings: Vec<Binding>,
        in_expr: Box<Expression>,
    },
    /// If-then-else
    IfThenElse {
        condition: Box<Expression>,
        then_expr: Box<Expression>,
        else_expr: Box<Expression>,
    },
    /// Pattern matching
    PatternMatch {
        input: Box<Expression>,
        cases: Vec<PatternCase>,
    },
    /// Record construction
    Record { fields: Vec<RecordField> },
    /// Record field access
    FieldAccess {
        record: Box<Expression>,
        field: String,
    },
    /// Tuple construction
    Tuple { elements: Vec<Expression> },
    /// Unit value
    Unit,
}

/// Literal value
///
/// Note: This type implements `PartialEq` but not `Eq` because `f64` (used in `Float`)
/// cannot implement `Eq` due to NaN values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Literal {
    /// Boolean literal
    Bool(bool),
    /// Integer literal
    Int(i64),
    /// Float literal
    Float(f64),
    /// String literal
    String(String),
    /// Character literal
    Char(char),
}

/// Binding in a let expression
///
/// Note: This type implements `PartialEq` but not `Eq` because it contains `Expression`
/// which cannot implement `Eq` due to `f64` values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    /// Variable name
    pub name: String,
    /// Bound expression
    pub expr: Expression,
}

/// Pattern case in pattern matching
///
/// Note: This type implements `PartialEq` but not `Eq` because it contains `Pattern` and `Expression`
/// which cannot implement `Eq` due to `f64` values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternCase {
    /// Pattern to match
    pub pattern: Pattern,
    /// Expression to evaluate if pattern matches
    pub expr: Expression,
}

/// Pattern in pattern matching
///
/// Note: This type implements `PartialEq` but not `Eq` because it contains `Literal`
/// which cannot implement `Eq` due to `f64` values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Pattern {
    /// Wildcard pattern
    Wildcard,
    /// Variable pattern
    Variable { name: String },
    /// Literal pattern
    Literal { value: Literal },
    /// Constructor pattern
    Constructor {
        name: String,
        arguments: Vec<Pattern>,
    },
    /// Tuple pattern
    Tuple { elements: Vec<Pattern> },
    /// Record pattern
    Record { fields: Vec<String> },
    /// Unit pattern
    Unit,
}

/// Record field in record construction
///
/// Note: This type implements `PartialEq` but not `Eq` because it contains `Expression`
/// which cannot implement `Eq` due to `f64` values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordField {
    /// Field name
    pub name: String,
    /// Field value expression
    pub expr: Expression,
}

/// Functional utilities for working with Morphir IR
impl Package {
    /// Create a new empty package
    pub fn new(name: String) -> Self {
        Self {
            name,
            modules: Vec::new(),
        }
    }

    /// Add a module to the package (returns a new package)
    pub fn with_module(mut self, module: Module) -> Self {
        self.modules.push(module);
        self
    }
}

impl Module {
    /// Create a new empty module
    pub fn new(name: String) -> Self {
        Self {
            name,
            types: Vec::new(),
            values: Vec::new(),
        }
    }

    /// Add a type definition (returns a new module)
    pub fn with_type(mut self, typ: TypeDefinition) -> Self {
        self.types.push(typ);
        self
    }

    /// Add a value definition (returns a new module)
    pub fn with_value(mut self, value: ValueDefinition) -> Self {
        self.values.push(value);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_creation() {
        let package = Package::new("TestPackage".to_string());
        assert_eq!(package.name, "TestPackage");
        assert_eq!(package.modules.len(), 0);
    }

    #[test]
    fn test_module_creation() {
        let module = Module::new("TestModule".to_string());
        assert_eq!(module.name, "TestModule");
        assert_eq!(module.types.len(), 0);
        assert_eq!(module.values.len(), 0);
    }
}


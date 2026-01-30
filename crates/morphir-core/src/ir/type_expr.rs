//! Type expressions for Morphir IR.
//!
//! This module defines the `Type` enum which represents type expressions
//! in the Morphir IR. Type attributes are always `TypeAttributes` (V4 format).
//!
//! # Examples
//!
//! ```rust,ignore
//! let t: Type = Type::Unit(TypeAttributes::default());
//! ```

use super::attributes::TypeAttributes;
use crate::naming::{FQName, Name};

/// A type expression with V4 attributes.
///
/// Type expressions form the type system of Morphir IR. Each variant
/// carries `TypeAttributes` which can store metadata like
/// source locations, type constraints, or extensions.
///
/// # Examples
///
/// ```rust,ignore
/// let t: Type = Type::Unit(TypeAttributes::default());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// Type variable (generic type parameter)
    ///
    /// Example: `a` in `List a`
    Variable(TypeAttributes, Name),

    /// Reference to a named type
    ///
    /// Example: `List Int` is `Reference(_, fqname_of_list, [Type::Reference(_, fqname_of_int, [])])`
    Reference(TypeAttributes, FQName, Vec<Type>),

    /// Tuple type (product type with positional elements)
    ///
    /// Example: `(Int, String, Bool)`
    Tuple(TypeAttributes, Vec<Type>),

    /// Record type (product type with named fields)
    ///
    /// Example: `{ name : String, age : Int }`
    Record(TypeAttributes, Vec<Field>),

    /// Extensible record type (record with a row variable)
    ///
    /// Example: `{ a | name : String }` where `a` is the row variable
    ExtensibleRecord(TypeAttributes, Name, Vec<Field>),

    /// Function type (arrow type)
    ///
    /// Example: `Int -> String`
    Function(TypeAttributes, Box<Type>, Box<Type>),

    /// Unit type (empty tuple, void equivalent)
    ///
    /// Example: `()`
    Unit(TypeAttributes),
}

/// A field in a record type.
///
/// Fields have a name and a type.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// The name of the field
    pub name: Name,
    /// The type of the field
    pub tpe: Type,
}

impl Type {
    /// Get the attributes of this type
    pub fn attributes(&self) -> &TypeAttributes {
        match self {
            Type::Variable(a, _) => a,
            Type::Reference(a, _, _) => a,
            Type::Tuple(a, _) => a,
            Type::Record(a, _) => a,
            Type::ExtensibleRecord(a, _, _) => a,
            Type::Function(a, _, _) => a,
            Type::Unit(a) => a,
        }
    }

    /// Create a variable type
    pub fn variable(attrs: TypeAttributes, name: Name) -> Self {
        Type::Variable(attrs, name)
    }

    /// Create a reference type
    pub fn reference(attrs: TypeAttributes, fqname: FQName, type_params: Vec<Type>) -> Self {
        Type::Reference(attrs, fqname, type_params)
    }

    /// Create a tuple type
    pub fn tuple(attrs: TypeAttributes, elements: Vec<Type>) -> Self {
        Type::Tuple(attrs, elements)
    }

    /// Create a record type
    pub fn record(attrs: TypeAttributes, fields: Vec<Field>) -> Self {
        Type::Record(attrs, fields)
    }

    /// Create an extensible record type
    pub fn extensible_record(attrs: TypeAttributes, variable: Name, fields: Vec<Field>) -> Self {
        Type::ExtensibleRecord(attrs, variable, fields)
    }

    /// Create a function type
    pub fn function(attrs: TypeAttributes, arg: Type, result: Type) -> Self {
        Type::Function(attrs, Box::new(arg), Box::new(result))
    }

    /// Create a unit type
    pub fn unit(attrs: TypeAttributes) -> Self {
        Type::Unit(attrs)
    }
}

impl Field {
    /// Create a new field
    pub fn new(name: Name, tpe: Type) -> Self {
        Field { name, tpe }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variable_type() {
        let var: Type = Type::variable(TypeAttributes::default(), Name::from("a"));
        assert!(matches!(var, Type::Variable(_, _)));
    }

    #[test]
    fn test_unit_type() {
        let unit: Type = Type::unit(TypeAttributes::default());
        assert!(matches!(unit, Type::Unit(_)));
    }

    #[test]
    fn test_function_type() {
        let func: Type = Type::function(
            TypeAttributes::default(),
            Type::unit(TypeAttributes::default()),
            Type::unit(TypeAttributes::default()),
        );
        assert!(matches!(func, Type::Function(_, _, _)));
    }
}

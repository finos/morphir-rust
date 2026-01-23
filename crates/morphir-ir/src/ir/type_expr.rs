//! Type expressions for Morphir IR.
//!
//! This module defines the `Type<A>` enum which represents type expressions
//! in the Morphir IR. The type parameter `A` represents the attributes
//! associated with each type node.
//!
//! # Default Type Parameter
//!
//! V4 is the default format - `Type` without type parameters uses `TypeAttributes`:
//! ```rust,ignore
//! let t: Type = Type::Unit(TypeAttributes::default());  // V4 type
//! let t: Type<serde_json::Value> = Type::Unit(json!({}));  // Classic type
//! ```

use crate::naming::{FQName, Name};
use super::attributes::TypeAttributes;

/// A type expression with generic attributes.
///
/// Type expressions form the type system of Morphir IR. Each variant
/// carries attributes of type `A` which can store metadata like
/// source locations, type constraints, or extensions.
///
/// # Type Parameters
/// - `A`: The type of attributes attached to each type node.
///        Defaults to `TypeAttributes` (V4 format).
///
/// # Examples
///
/// ```rust,ignore
/// // V4 format (default) - uses TypeAttributes
/// let t: Type = Type::Unit(TypeAttributes::default());
///
/// // Classic format - explicit serde_json::Value
/// let t: Type<serde_json::Value> = Type::Unit(serde_json::json!({}));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum Type<A: Clone = TypeAttributes> {
    /// Type variable (generic type parameter)
    ///
    /// Example: `a` in `List a`
    Variable(A, Name),

    /// Reference to a named type
    ///
    /// Example: `List Int` is `Reference(_, fqname_of_list, [Type::Reference(_, fqname_of_int, [])])`
    Reference(A, FQName, Vec<Type<A>>),

    /// Tuple type (product type with positional elements)
    ///
    /// Example: `(Int, String, Bool)`
    Tuple(A, Vec<Type<A>>),

    /// Record type (product type with named fields)
    ///
    /// Example: `{ name : String, age : Int }`
    Record(A, Vec<Field<A>>),

    /// Extensible record type (record with a row variable)
    ///
    /// Example: `{ a | name : String }` where `a` is the row variable
    ExtensibleRecord(A, Name, Vec<Field<A>>),

    /// Function type (arrow type)
    ///
    /// Example: `Int -> String`
    Function(A, Box<Type<A>>, Box<Type<A>>),

    /// Unit type (empty tuple, void equivalent)
    ///
    /// Example: `()`
    Unit(A),
}

/// A field in a record type.
///
/// Fields have a name and a type.
///
/// # Type Parameters
/// - `A`: The type of attributes attached to the field's type.
///        Defaults to `TypeAttributes` (V4 format).
#[derive(Debug, Clone, PartialEq)]
pub struct Field<A: Clone = TypeAttributes> {
    /// The name of the field
    pub name: Name,
    /// The type of the field
    pub tpe: Type<A>,
}

impl<A: Clone> Type<A> {
    /// Get the attributes of this type
    pub fn attributes(&self) -> &A {
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
    pub fn variable(attrs: A, name: Name) -> Self {
        Type::Variable(attrs, name)
    }

    /// Create a reference type
    pub fn reference(attrs: A, fqname: FQName, type_params: Vec<Type<A>>) -> Self {
        Type::Reference(attrs, fqname, type_params)
    }

    /// Create a tuple type
    pub fn tuple(attrs: A, elements: Vec<Type<A>>) -> Self {
        Type::Tuple(attrs, elements)
    }

    /// Create a record type
    pub fn record(attrs: A, fields: Vec<Field<A>>) -> Self {
        Type::Record(attrs, fields)
    }

    /// Create an extensible record type
    pub fn extensible_record(attrs: A, variable: Name, fields: Vec<Field<A>>) -> Self {
        Type::ExtensibleRecord(attrs, variable, fields)
    }

    /// Create a function type
    pub fn function(attrs: A, arg: Type<A>, result: Type<A>) -> Self {
        Type::Function(attrs, Box::new(arg), Box::new(result))
    }

    /// Create a unit type
    pub fn unit(attrs: A) -> Self {
        Type::Unit(attrs)
    }
}

impl<A: Clone> Type<A> {
    /// Map a function over the attributes of this type and all nested types
    pub fn map_attributes<B: Clone, F>(&self, f: &F) -> Type<B>
    where
        F: Fn(&A) -> B,
    {
        match self {
            Type::Variable(a, name) => Type::Variable(f(a), name.clone()),
            Type::Reference(a, fqname, params) => Type::Reference(
                f(a),
                fqname.clone(),
                params.iter().map(|p| p.map_attributes(f)).collect(),
            ),
            Type::Tuple(a, elements) => {
                Type::Tuple(f(a), elements.iter().map(|e| e.map_attributes(f)).collect())
            }
            Type::Record(a, fields) => Type::Record(
                f(a),
                fields
                    .iter()
                    .map(|field| Field {
                        name: field.name.clone(),
                        tpe: field.tpe.map_attributes(f),
                    })
                    .collect(),
            ),
            Type::ExtensibleRecord(a, var, fields) => Type::ExtensibleRecord(
                f(a),
                var.clone(),
                fields
                    .iter()
                    .map(|field| Field {
                        name: field.name.clone(),
                        tpe: field.tpe.map_attributes(f),
                    })
                    .collect(),
            ),
            Type::Function(a, arg, result) => Type::Function(
                f(a),
                Box::new(arg.map_attributes(f)),
                Box::new(result.map_attributes(f)),
            ),
            Type::Unit(a) => Type::Unit(f(a)),
        }
    }
}

impl<A: Clone> Field<A> {
    /// Create a new field
    pub fn new(name: Name, tpe: Type<A>) -> Self {
        Field { name, tpe }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_attrs() -> () {
        ()
    }

    #[test]
    fn test_variable_type() {
        let var = Type::variable(test_attrs(), Name::from("a"));
        assert!(matches!(var, Type::Variable(_, _)));
    }

    #[test]
    fn test_unit_type() {
        let unit = Type::unit(test_attrs());
        assert!(matches!(unit, Type::Unit(_)));
    }

    #[test]
    fn test_function_type() {
        let func = Type::function(
            test_attrs(),
            Type::unit(test_attrs()),
            Type::unit(test_attrs()),
        );
        assert!(matches!(func, Type::Function(_, _, _)));
    }

    #[test]
    fn test_map_attributes() {
        let var: Type<i32> = Type::Variable(1, Name::from("a"));
        let mapped: Type<String> = var.map_attributes(&|n| n.to_string());
        assert!(matches!(mapped, Type::Variable(s, _) if s == "1"));
    }
}

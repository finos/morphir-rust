//! Type definitions for Morphir IR.
//!
//! This module defines type definitions including type aliases and custom types
//! (algebraic data types). V4 adds IncompleteTypeDefinition for error recovery.
//!
//! # Default Type Parameter
//!
//! V4 is the default format - `TypeDefinition` without type parameters uses `TypeAttributes`:
//! ```rust,ignore
//! let def: TypeDefinition = TypeDefinition::type_alias(...);  // V4
//! let def: TypeDefinition<serde_json::Value> = ...;  // Classic
//! ```

use crate::naming::Name;
use super::attributes::TypeAttributes;
use super::type_expr::Type;
use super::value_expr::HoleReason;

/// A type definition with generic attributes.
///
/// Type definitions describe the shape of types in the Morphir IR.
/// V4 adds IncompleteTypeDefinition for incremental compilation.
///
/// # Type Parameters
/// - `A`: The type of attributes attached to type nodes.
///        Defaults to `TypeAttributes` (V4 format).
#[derive(Debug, Clone, PartialEq)]
pub enum TypeDefinition<A: Clone = TypeAttributes> {
    /// Type alias definition
    ///
    /// Example: `type alias Person = { name : String, age : Int }`
    TypeAliasDefinition {
        /// Type parameters (e.g., `a` in `type alias Maybe a = ...`)
        type_params: Vec<Name>,
        /// The type this alias expands to
        type_expr: Type<A>,
    },

    /// Custom type (algebraic data type) definition
    ///
    /// Example: `type Maybe a = Just a | Nothing`
    CustomTypeDefinition {
        /// Type parameters
        type_params: Vec<Name>,
        /// Access control for the constructors
        access_controlled_ctors: AccessControlled<Vec<Constructor<A>>>,
    },

    /// Incomplete type definition (V4 only)
    ///
    /// Represents types that couldn't be fully resolved.
    /// Used for incremental compilation and error recovery.
    IncompleteTypeDefinition {
        /// Type parameters
        type_params: Vec<Name>,
        /// Reason for incompleteness
        incompleteness: Incompleteness,
    },
}

/// Access control wrapper
///
/// Controls visibility of type constructors or other definitions.
#[derive(Debug, Clone, PartialEq)]
pub enum AccessControlled<T> {
    /// Publicly accessible
    Public(T),
    /// Only accessible within the module
    Private(T),
}

/// A constructor for a custom type
///
/// Example: `Just` in `type Maybe a = Just a | Nothing`
#[derive(Debug, Clone, PartialEq)]
pub struct Constructor<A: Clone = TypeAttributes> {
    /// The name of the constructor (e.g., `Just`)
    pub name: Name,
    /// The arguments to the constructor with their names and types
    pub args: Vec<ConstructorArg<A>>,
}

/// Constructor argument tuple struct: (name, type)
///
/// More ergonomic than `(Name, Type<A>)`.
#[derive(Debug, Clone, PartialEq)]
pub struct ConstructorArg<A: Clone = TypeAttributes>(pub Name, pub Type<A>);

/// Reason for an incomplete type (V4 only)
#[derive(Debug, Clone, PartialEq)]
pub enum Incompleteness {
    /// Type has unresolved dependencies or errors
    Hole(HoleReason),
    /// Type is work in progress
    Draft,
}

impl<A: Clone> TypeDefinition<A> {
    /// Create a type alias definition
    pub fn type_alias(type_params: Vec<Name>, type_expr: Type<A>) -> Self {
        TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr,
        }
    }

    /// Create a custom type definition with public constructors
    pub fn custom_type_public(type_params: Vec<Name>, constructors: Vec<Constructor<A>>) -> Self {
        TypeDefinition::CustomTypeDefinition {
            type_params,
            access_controlled_ctors: AccessControlled::Public(constructors),
        }
    }

    /// Create a custom type definition with private constructors (opaque type)
    pub fn custom_type_private(type_params: Vec<Name>, constructors: Vec<Constructor<A>>) -> Self {
        TypeDefinition::CustomTypeDefinition {
            type_params,
            access_controlled_ctors: AccessControlled::Private(constructors),
        }
    }

    /// Create an incomplete type definition (V4 only)
    pub fn incomplete(type_params: Vec<Name>, incompleteness: Incompleteness) -> Self {
        TypeDefinition::IncompleteTypeDefinition {
            type_params,
            incompleteness,
        }
    }

    /// Get the type parameters of this definition
    pub fn type_params(&self) -> &[Name] {
        match self {
            TypeDefinition::TypeAliasDefinition { type_params, .. } => type_params,
            TypeDefinition::CustomTypeDefinition { type_params, .. } => type_params,
            TypeDefinition::IncompleteTypeDefinition { type_params, .. } => type_params,
        }
    }
}

impl<T> AccessControlled<T> {
    /// Create a public access-controlled value
    pub fn public(value: T) -> Self {
        AccessControlled::Public(value)
    }

    /// Create a private access-controlled value
    pub fn private(value: T) -> Self {
        AccessControlled::Private(value)
    }

    /// Get the inner value regardless of access level
    pub fn value(&self) -> &T {
        match self {
            AccessControlled::Public(v) => v,
            AccessControlled::Private(v) => v,
        }
    }

    /// Check if this is public
    pub fn is_public(&self) -> bool {
        matches!(self, AccessControlled::Public(_))
    }

    /// Check if this is private
    pub fn is_private(&self) -> bool {
        matches!(self, AccessControlled::Private(_))
    }

    /// Map a function over the inner value
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> AccessControlled<U> {
        match self {
            AccessControlled::Public(v) => AccessControlled::Public(f(v)),
            AccessControlled::Private(v) => AccessControlled::Private(f(v)),
        }
    }
}

impl<A: Clone> Constructor<A> {
    /// Create a new constructor
    pub fn new(name: Name, args: Vec<ConstructorArg<A>>) -> Self {
        Constructor { name, args }
    }

    /// Create a constructor with no arguments (constant)
    pub fn constant(name: Name) -> Self {
        Constructor {
            name,
            args: vec![],
        }
    }
}

impl<A: Clone> ConstructorArg<A> {
    /// Create a new constructor argument
    pub fn new(name: Name, tpe: Type<A>) -> Self {
        ConstructorArg(name, tpe)
    }

    /// Get the name
    pub fn name(&self) -> &Name {
        &self.0
    }

    /// Get the type
    pub fn tpe(&self) -> &Type<A> {
        &self.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::type_expr::Type;

    fn test_attrs() -> () {
        ()
    }

    #[test]
    fn test_type_alias_definition() {
        let def: TypeDefinition<()> = TypeDefinition::type_alias(
            vec![],
            Type::unit(test_attrs()),
        );
        assert!(matches!(def, TypeDefinition::TypeAliasDefinition { .. }));
    }

    #[test]
    fn test_custom_type_definition() {
        let nothing = Constructor::constant(Name::from("Nothing"));
        let just = Constructor::new(
            Name::from("Just"),
            vec![ConstructorArg::new(Name::from("value"), Type::variable(test_attrs(), Name::from("a")))],
        );

        let def: TypeDefinition<()> = TypeDefinition::custom_type_public(
            vec![Name::from("a")],
            vec![just, nothing],
        );

        if let TypeDefinition::CustomTypeDefinition { type_params, access_controlled_ctors } = def {
            assert_eq!(type_params.len(), 1);
            assert!(access_controlled_ctors.is_public());
            assert_eq!(access_controlled_ctors.value().len(), 2);
        } else {
            panic!("Expected CustomTypeDefinition");
        }
    }

    #[test]
    fn test_opaque_type() {
        let def: TypeDefinition<()> = TypeDefinition::custom_type_private(
            vec![],
            vec![Constructor::constant(Name::from("Internal"))],
        );

        if let TypeDefinition::CustomTypeDefinition { access_controlled_ctors, .. } = def {
            assert!(access_controlled_ctors.is_private());
        } else {
            panic!("Expected CustomTypeDefinition");
        }
    }

    #[test]
    fn test_incomplete_type_definition() {
        let def: TypeDefinition<()> = TypeDefinition::incomplete(
            vec![Name::from("a")],
            Incompleteness::Draft,
        );

        assert!(matches!(
            def,
            TypeDefinition::IncompleteTypeDefinition {
                incompleteness: Incompleteness::Draft,
                ..
            }
        ));
    }

    #[test]
    fn test_access_controlled_map() {
        let public: AccessControlled<i32> = AccessControlled::public(42);
        let mapped = public.map(|x| x.to_string());
        assert!(matches!(mapped, AccessControlled::Public(s) if s == "42"));
    }
}

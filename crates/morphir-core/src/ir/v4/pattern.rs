//! Pattern matching constructs for Morphir IR.
//!
//! This module defines the `Pattern` enum which represents patterns
//! used in pattern matching expressions. Patterns always use `ValueAttributes`
//! since they appear in value contexts.
//!
//! # Examples
//!
//! ```rust,ignore
//! let p: Pattern = Pattern::WildcardPattern(ValueAttributes::default());
//! ```

use super::attributes::ValueAttributes;
use super::literal::Literal;
use crate::naming::{FQName, Name};

/// A pattern with V4 value attributes.
///
/// Patterns are used in pattern matching expressions to destructure
/// values and bind variables. Each variant carries `ValueAttributes`.
///
/// # Examples
///
/// ```rust,ignore
/// let p: Pattern = Pattern::WildcardPattern(ValueAttributes::default());
/// ```
// The variant names include "Pattern" suffix as per the Morphir specification
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// Wildcard pattern that matches anything
    ///
    /// Example: `_` in `case x of _ -> ...`
    WildcardPattern(ValueAttributes),

    /// As-pattern that binds a name to a matched value
    ///
    /// Example: `x as (a, b)` binds `x` to the whole tuple
    AsPattern(ValueAttributes, Box<Pattern>, Name),

    /// Tuple pattern for destructuring tuples
    ///
    /// Example: `(a, b, c)` in `case x of (a, b, c) -> ...`
    TuplePattern(ValueAttributes, Vec<Pattern>),

    /// Constructor pattern for matching algebraic data types
    ///
    /// Example: `Just x` in `case maybe of Just x -> ...`
    ConstructorPattern(ValueAttributes, FQName, Vec<Pattern>),

    /// Empty list pattern
    ///
    /// Example: `[]` in `case list of [] -> ...`
    EmptyListPattern(ValueAttributes),

    /// Head-tail pattern for list destructuring
    ///
    /// Example: `head :: tail` in `case list of head :: tail -> ...`
    HeadTailPattern(ValueAttributes, Box<Pattern>, Box<Pattern>),

    /// Literal pattern for matching constant values
    ///
    /// Example: `42` in `case x of 42 -> ...`
    LiteralPattern(ValueAttributes, Literal),

    /// Unit pattern
    ///
    /// Example: `()` in `case x of () -> ...`
    UnitPattern(ValueAttributes),
}

impl Pattern {
    /// Get the attributes of this pattern
    pub fn attributes(&self) -> &ValueAttributes {
        match self {
            Pattern::WildcardPattern(a) => a,
            Pattern::AsPattern(a, _, _) => a,
            Pattern::TuplePattern(a, _) => a,
            Pattern::ConstructorPattern(a, _, _) => a,
            Pattern::EmptyListPattern(a) => a,
            Pattern::HeadTailPattern(a, _, _) => a,
            Pattern::LiteralPattern(a, _) => a,
            Pattern::UnitPattern(a) => a,
        }
    }

    /// Create a wildcard pattern
    pub fn wildcard(attrs: ValueAttributes) -> Self {
        Pattern::WildcardPattern(attrs)
    }

    /// Create an as-pattern
    pub fn as_pattern(attrs: ValueAttributes, pattern: Pattern, name: Name) -> Self {
        Pattern::AsPattern(attrs, Box::new(pattern), name)
    }

    /// Create a tuple pattern
    pub fn tuple(attrs: ValueAttributes, elements: Vec<Pattern>) -> Self {
        Pattern::TuplePattern(attrs, elements)
    }

    /// Create a constructor pattern
    pub fn constructor(attrs: ValueAttributes, name: FQName, args: Vec<Pattern>) -> Self {
        Pattern::ConstructorPattern(attrs, name, args)
    }

    /// Create an empty list pattern
    pub fn empty_list(attrs: ValueAttributes) -> Self {
        Pattern::EmptyListPattern(attrs)
    }

    /// Create a head-tail pattern
    pub fn head_tail(attrs: ValueAttributes, head: Pattern, tail: Pattern) -> Self {
        Pattern::HeadTailPattern(attrs, Box::new(head), Box::new(tail))
    }

    /// Create a literal pattern
    pub fn literal(attrs: ValueAttributes, lit: Literal) -> Self {
        Pattern::LiteralPattern(attrs, lit)
    }

    /// Create a unit pattern
    pub fn unit(attrs: ValueAttributes) -> Self {
        Pattern::UnitPattern(attrs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wildcard_pattern() {
        let p: Pattern = Pattern::wildcard(ValueAttributes::default());
        assert!(matches!(p, Pattern::WildcardPattern(_)));
    }

    #[test]
    fn test_tuple_pattern() {
        let p: Pattern = Pattern::tuple(
            ValueAttributes::default(),
            vec![
                Pattern::wildcard(ValueAttributes::default()),
                Pattern::wildcard(ValueAttributes::default()),
            ],
        );
        assert!(matches!(p, Pattern::TuplePattern(_, elements) if elements.len() == 2));
    }

    #[test]
    fn test_literal_pattern() {
        let p: Pattern = Pattern::literal(ValueAttributes::default(), Literal::Integer(42));
        assert!(matches!(
            p,
            Pattern::LiteralPattern(_, Literal::Integer(42))
        ));
    }
}

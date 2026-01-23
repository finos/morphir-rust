//! Pattern matching constructs for Morphir IR.
//!
//! This module defines the `Pattern<A>` enum which represents patterns
//! used in pattern matching expressions.
//!
//! # Default Type Parameter
//!
//! V4 is the default format - `Pattern` without type parameters uses `ValueAttributes`:
//! ```rust,ignore
//! let p: Pattern = Pattern::WildcardPattern(ValueAttributes::default());  // V4
//! let p: Pattern<serde_json::Value> = Pattern::WildcardPattern(json!({}));  // Classic
//! ```

use super::attributes::ValueAttributes;
use super::literal::Literal;
use crate::naming::{FQName, Name};

/// A pattern with generic attributes.
///
/// Patterns are used in pattern matching expressions to destructure
/// values and bind variables. Each variant carries attributes of type `A`.
///
/// # Type Parameters
/// - `A`: The type of attributes attached to each pattern node.
///        Defaults to `ValueAttributes` (V4 format) since patterns
///        appear in value contexts.
///
/// # Examples
///
/// ```rust,ignore
/// // V4 format (default) - uses ValueAttributes
/// let p: Pattern = Pattern::WildcardPattern(ValueAttributes::default());
///
/// // Classic format - explicit serde_json::Value
/// let p: Pattern<serde_json::Value> = Pattern::WildcardPattern(serde_json::json!({}));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern<A: Clone = ValueAttributes> {
    /// Wildcard pattern that matches anything
    ///
    /// Example: `_` in `case x of _ -> ...`
    WildcardPattern(A),

    /// As-pattern that binds a name to a matched value
    ///
    /// Example: `x as (a, b)` binds `x` to the whole tuple
    AsPattern(A, Box<Pattern<A>>, Name),

    /// Tuple pattern for destructuring tuples
    ///
    /// Example: `(a, b, c)` in `case x of (a, b, c) -> ...`
    TuplePattern(A, Vec<Pattern<A>>),

    /// Constructor pattern for matching algebraic data types
    ///
    /// Example: `Just x` in `case maybe of Just x -> ...`
    ConstructorPattern(A, FQName, Vec<Pattern<A>>),

    /// Empty list pattern
    ///
    /// Example: `[]` in `case list of [] -> ...`
    EmptyListPattern(A),

    /// Head-tail pattern for list destructuring
    ///
    /// Example: `head :: tail` in `case list of head :: tail -> ...`
    HeadTailPattern(A, Box<Pattern<A>>, Box<Pattern<A>>),

    /// Literal pattern for matching constant values
    ///
    /// Example: `42` in `case x of 42 -> ...`
    LiteralPattern(A, Literal),

    /// Unit pattern
    ///
    /// Example: `()` in `case x of () -> ...`
    UnitPattern(A),
}

impl<A: Clone> Pattern<A> {
    /// Get the attributes of this pattern
    pub fn attributes(&self) -> &A {
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
    pub fn wildcard(attrs: A) -> Self {
        Pattern::WildcardPattern(attrs)
    }

    /// Create an as-pattern
    pub fn as_pattern(attrs: A, pattern: Pattern<A>, name: Name) -> Self {
        Pattern::AsPattern(attrs, Box::new(pattern), name)
    }

    /// Create a tuple pattern
    pub fn tuple(attrs: A, elements: Vec<Pattern<A>>) -> Self {
        Pattern::TuplePattern(attrs, elements)
    }

    /// Create a constructor pattern
    pub fn constructor(attrs: A, name: FQName, args: Vec<Pattern<A>>) -> Self {
        Pattern::ConstructorPattern(attrs, name, args)
    }

    /// Create an empty list pattern
    pub fn empty_list(attrs: A) -> Self {
        Pattern::EmptyListPattern(attrs)
    }

    /// Create a head-tail pattern
    pub fn head_tail(attrs: A, head: Pattern<A>, tail: Pattern<A>) -> Self {
        Pattern::HeadTailPattern(attrs, Box::new(head), Box::new(tail))
    }

    /// Create a literal pattern
    pub fn literal(attrs: A, lit: Literal) -> Self {
        Pattern::LiteralPattern(attrs, lit)
    }

    /// Create a unit pattern
    pub fn unit(attrs: A) -> Self {
        Pattern::UnitPattern(attrs)
    }
}

impl<A: Clone> Pattern<A> {
    /// Map a function over the attributes of this pattern and all nested patterns
    pub fn map_attributes<B: Clone, F>(&self, f: &F) -> Pattern<B>
    where
        F: Fn(&A) -> B,
    {
        match self {
            Pattern::WildcardPattern(a) => Pattern::WildcardPattern(f(a)),
            Pattern::AsPattern(a, pattern, name) => {
                Pattern::AsPattern(f(a), Box::new(pattern.map_attributes(f)), name.clone())
            }
            Pattern::TuplePattern(a, elements) => {
                Pattern::TuplePattern(f(a), elements.iter().map(|p| p.map_attributes(f)).collect())
            }
            Pattern::ConstructorPattern(a, name, args) => Pattern::ConstructorPattern(
                f(a),
                name.clone(),
                args.iter().map(|p| p.map_attributes(f)).collect(),
            ),
            Pattern::EmptyListPattern(a) => Pattern::EmptyListPattern(f(a)),
            Pattern::HeadTailPattern(a, head, tail) => Pattern::HeadTailPattern(
                f(a),
                Box::new(head.map_attributes(f)),
                Box::new(tail.map_attributes(f)),
            ),
            Pattern::LiteralPattern(a, lit) => Pattern::LiteralPattern(f(a), lit.clone()),
            Pattern::UnitPattern(a) => Pattern::UnitPattern(f(a)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_attrs() -> () {
        ()
    }

    #[test]
    fn test_wildcard_pattern() {
        let p = Pattern::wildcard(test_attrs());
        assert!(matches!(p, Pattern::WildcardPattern(_)));
    }

    #[test]
    fn test_tuple_pattern() {
        let p = Pattern::tuple(
            test_attrs(),
            vec![
                Pattern::wildcard(test_attrs()),
                Pattern::wildcard(test_attrs()),
            ],
        );
        assert!(matches!(p, Pattern::TuplePattern(_, elements) if elements.len() == 2));
    }

    #[test]
    fn test_literal_pattern() {
        let p = Pattern::literal(test_attrs(), Literal::Integer(42));
        assert!(matches!(
            p,
            Pattern::LiteralPattern(_, Literal::Integer(42))
        ));
    }

    #[test]
    fn test_map_attributes() {
        let p: Pattern<i32> = Pattern::WildcardPattern(1);
        let mapped: Pattern<String> = p.map_attributes(&|n| n.to_string());
        assert!(matches!(mapped, Pattern::WildcardPattern(s) if s == "1"));
    }
}

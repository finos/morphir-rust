//! Classic IR Pattern types
//!
//! Pattern matching for the Classic Morphir IR format.

use super::naming::{FQName, Name};
use serde::de::{self, IgnoredAny, SeqAccess, Visitor};
use serde::ser::{SerializeTuple, Serializer};
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::fmt;

use super::literal::Literal;

/// Pattern for pattern matching
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern<A> {
    Wildcard(A),
    As(A, Box<Pattern<A>>, Name),
    Tuple(A, Vec<Pattern<A>>),
    Constructor(A, FQName, Vec<Pattern<A>>),
    EmptyList(A),
    HeadTail(A, Box<Pattern<A>>, Box<Pattern<A>>),
    Literal(A, Literal),
    Unit(A),
    Variable(A, Name),
}

impl<A: Serialize> Serialize for Pattern<A> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Pattern::Wildcard(a) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("WildcardPattern")?;
                tuple.serialize_element(a)?;
                tuple.end()
            }
            Pattern::As(a, pattern, name) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("AsPattern")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(pattern)?;
                tuple.serialize_element(name)?;
                tuple.end()
            }
            Pattern::Tuple(a, patterns) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("TuplePattern")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(patterns)?;
                tuple.end()
            }
            Pattern::Constructor(a, name, args) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("ConstructorPattern")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(name)?;
                tuple.serialize_element(args)?;
                tuple.end()
            }
            Pattern::EmptyList(a) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("EmptyListPattern")?;
                tuple.serialize_element(a)?;
                tuple.end()
            }
            Pattern::HeadTail(a, head, tail) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element("HeadTailPattern")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(head)?;
                tuple.serialize_element(tail)?;
                tuple.end()
            }
            Pattern::Literal(a, lit) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("LiteralPattern")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(lit)?;
                tuple.end()
            }
            Pattern::Unit(a) => {
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element("UnitPattern")?;
                tuple.serialize_element(a)?;
                tuple.end()
            }
            Pattern::Variable(a, name) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element("VariablePattern")?;
                tuple.serialize_element(a)?;
                tuple.serialize_element(name)?;
                tuple.end()
            }
        }
    }
}

impl<'de, A: Deserialize<'de>> Deserialize<'de> for Pattern<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PatternVisitor<A>(std::marker::PhantomData<A>);

        impl<'de, A: Deserialize<'de>> Visitor<'de> for PatternVisitor<A> {
            type Value = Pattern<A>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a classic Pattern array")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let tag: Cow<'de, str> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;

                match tag.as_ref() {
                    "WildcardPattern" | "wildcard" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                        if seq.next_element::<IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom("Expected end of WildcardPattern array"));
                        }

                        Ok(Pattern::Wildcard(a))
                    }
                    "AsPattern" | "as_pattern" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let pattern = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;

                        if seq.next_element::<IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom("Expected end of AsPattern array"));
                        }

                        Ok(Pattern::As(a, pattern, name))
                    }
                    "TuplePattern" | "tuple_pattern" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let patterns = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;

                        if seq.next_element::<IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom("Expected end of TuplePattern array"));
                        }

                        Ok(Pattern::Tuple(a, patterns))
                    }
                    "ConstructorPattern" | "constructor_pattern" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let args = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;

                        if seq.next_element::<IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom(
                                "Expected end of ConstructorPattern array",
                            ));
                        }

                        Ok(Pattern::Constructor(a, name, args))
                    }
                    "EmptyListPattern" | "empty_list_pattern" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                        if seq.next_element::<IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom(
                                "Expected end of EmptyListPattern array",
                            ));
                        }

                        Ok(Pattern::EmptyList(a))
                    }
                    "HeadTailPattern" | "head_tail_pattern" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let head = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let tail = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;

                        if seq.next_element::<IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom("Expected end of HeadTailPattern array"));
                        }

                        Ok(Pattern::HeadTail(a, head, tail))
                    }
                    "LiteralPattern" | "literal_pattern" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let lit = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;

                        if seq.next_element::<IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom("Expected end of LiteralPattern array"));
                        }

                        Ok(Pattern::Literal(a, lit))
                    }
                    "UnitPattern" | "unit_pattern" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                        if seq.next_element::<IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom("Expected end of UnitPattern array"));
                        }

                        Ok(Pattern::Unit(a))
                    }
                    "VariablePattern" | "variable_pattern" => {
                        let a = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let name = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;

                        if seq.next_element::<IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom("Expected end of VariablePattern array"));
                        }

                        Ok(Pattern::Variable(a, name))
                    }
                    _ => Err(de::Error::unknown_variant(
                        tag.as_ref(),
                        &[
                            "WildcardPattern",
                            "AsPattern",
                            "TuplePattern",
                            "ConstructorPattern",
                            "EmptyListPattern",
                            "HeadTailPattern",
                            "LiteralPattern",
                            "UnitPattern",
                            "VariablePattern",
                        ],
                    )),
                }
            }
        }
        deserializer.deserialize_seq(PatternVisitor(std::marker::PhantomData))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::classic::naming::Path;

    #[test]
    fn test_serialize_pattern_wildcard() {
        let p: Pattern<()> = Pattern::Wildcard(());
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, r#"["WildcardPattern",null]"#);
        let deserialized: Pattern<()> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, p);
    }

    #[test]
    fn test_serialize_pattern_variable() {
        let p: Pattern<()> = Pattern::Variable((), Name::from_str("x"));
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, r#"["VariablePattern",null,["x"]]"#);
        let deserialized: Pattern<()> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, p);
    }

    #[test]
    fn test_serialize_pattern_as() {
        let p: Pattern<()> = Pattern::As((), Box::new(Pattern::Wildcard(())), Name::from_str("x"));
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, r#"["AsPattern",null,["WildcardPattern",null],["x"]]"#);
        let deserialized: Pattern<()> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, p);
    }

    #[test]
    fn test_serialize_pattern_tuple() {
        let p: Pattern<()> = Pattern::Tuple((), vec![Pattern::Wildcard(()), Pattern::Wildcard(())]);
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(
            json,
            r#"["TuplePattern",null,[["WildcardPattern",null],["WildcardPattern",null]]]"#
        );
        let deserialized: Pattern<()> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, p);
    }

    #[test]
    fn test_serialize_pattern_constructor() {
        let fq = FQName::new(
            Path::new(vec![Name::from_str("pkg")]),
            Path::new(vec![Name::from_str("mod")]),
            Name::from_str("ctor"),
        );
        let p: Pattern<()> = Pattern::Constructor((), fq, vec![Pattern::Wildcard(())]);
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(
            json,
            r#"["ConstructorPattern",null,[[["pkg"]],[["mod"]],["ctor"]],[["WildcardPattern",null]]]"#
        );
        let deserialized: Pattern<()> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, p);
    }

    #[test]
    fn test_serialize_pattern_empty_list() {
        let p: Pattern<()> = Pattern::EmptyList(());
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, r#"["EmptyListPattern",null]"#);
        let deserialized: Pattern<()> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, p);
    }

    #[test]
    fn test_serialize_pattern_head_tail() {
        let p: Pattern<()> = Pattern::HeadTail(
            (),
            Box::new(Pattern::Wildcard(())),
            Box::new(Pattern::Wildcard(())),
        );
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(
            json,
            r#"["HeadTailPattern",null,["WildcardPattern",null],["WildcardPattern",null]]"#
        );
        let deserialized: Pattern<()> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, p);
    }

    #[test]
    fn test_serialize_pattern_literal() {
        let p: Pattern<()> = Pattern::Literal((), Literal::WholeNumber(123));
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(
            json,
            r#"["LiteralPattern",null,["WholeNumberLiteral",123]]"#
        );
        let deserialized: Pattern<()> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, p);
    }

    #[test]
    fn test_serialize_pattern_unit() {
        let p: Pattern<()> = Pattern::Unit(());
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, r#"["UnitPattern",null]"#);
        let deserialized: Pattern<()> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, p);
    }
}

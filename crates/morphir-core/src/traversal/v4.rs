//! Read-only traversal for concrete Morphir IR v4 nodes.

use crate::ir::v4;
use crate::traversal::{CursorSegment, IrCursor};

pub trait V4Visitor {
    fn visit_type(&mut self, cursor: &mut IrCursor, value: &v4::Type) {
        walk_type(self, cursor, value);
    }

    fn visit_pattern(&mut self, cursor: &mut IrCursor, value: &v4::Pattern) {
        walk_pattern(self, cursor, value);
    }

    fn visit_value(&mut self, cursor: &mut IrCursor, value: &v4::Value) {
        walk_value(self, cursor, value);
    }

    fn visit_definition(&mut self, cursor: &mut IrCursor, value: &v4::ValueDefinition) {
        walk_definition(self, cursor, value);
    }
}

pub fn walk_type<V: V4Visitor + ?Sized>(visitor: &mut V, cursor: &mut IrCursor, value: &v4::Type) {
    match value {
        v4::Type::ExtensibleRecord(_, _, fields) | v4::Type::Record(_, fields) => {
            for field in fields {
                cursor.with_segment(CursorSegment::Field(field.name.to_string()), |cursor| {
                    visitor.visit_type(cursor, &field.tpe);
                });
            }
        }
        v4::Type::Function(_, argument, result) => {
            cursor.with_segment(CursorSegment::Argument(0), |cursor| {
                visitor.visit_type(cursor, argument);
            });
            cursor.with_segment(CursorSegment::Branch("result"), |cursor| {
                visitor.visit_type(cursor, result);
            });
        }
        v4::Type::Reference(_, _, arguments) | v4::Type::Tuple(_, arguments) => {
            for (index, argument) in arguments.iter().enumerate() {
                cursor.with_segment(CursorSegment::Argument(index), |cursor| {
                    visitor.visit_type(cursor, argument);
                });
            }
        }
        v4::Type::Unit(_) | v4::Type::Variable(_, _) => {}
    }
}

pub fn walk_pattern<V: V4Visitor + ?Sized>(
    visitor: &mut V,
    cursor: &mut IrCursor,
    value: &v4::Pattern,
) {
    match value {
        v4::Pattern::AsPattern(_, pattern, _) => visitor.visit_pattern(cursor, pattern),
        v4::Pattern::TuplePattern(_, patterns)
        | v4::Pattern::ConstructorPattern(_, _, patterns) => {
            for (index, pattern) in patterns.iter().enumerate() {
                cursor.with_segment(CursorSegment::Argument(index), |cursor| {
                    visitor.visit_pattern(cursor, pattern);
                });
            }
        }
        v4::Pattern::HeadTailPattern(_, head, tail) => {
            cursor.with_segment(CursorSegment::Branch("head"), |cursor| {
                visitor.visit_pattern(cursor, head);
            });
            cursor.with_segment(CursorSegment::Branch("tail"), |cursor| {
                visitor.visit_pattern(cursor, tail);
            });
        }
        v4::Pattern::WildcardPattern(_)
        | v4::Pattern::EmptyListPattern(_)
        | v4::Pattern::LiteralPattern(_, _)
        | v4::Pattern::UnitPattern(_) => {}
    }
}

pub fn walk_definition<V: V4Visitor + ?Sized>(
    visitor: &mut V,
    cursor: &mut IrCursor,
    definition: &v4::ValueDefinition,
) {
    for (index, input) in definition.input_types.values().enumerate() {
        cursor.with_segment(CursorSegment::Argument(index), |cursor| {
            visitor.visit_type(cursor, &input.input_type);
        });
    }
    if let Some(output) = &definition.output_type {
        cursor.with_segment(CursorSegment::Branch("output"), |cursor| {
            visitor.visit_type(cursor, output);
        });
    }
    match &definition.body {
        v4::ValueBody::Expression(value) => visitor.visit_value(cursor, value),
        // An external definition's fallback body and an incomplete definition's partial body are
        // ordinary expressions, so they are walked.
        v4::ValueBody::External {
            fallback: Some(value),
            ..
        }
        | v4::ValueBody::Incomplete {
            partial_body: Some(value),
            ..
        } => visitor.visit_value(cursor, value),
        // A native body has no expression at all, and an incomplete body's `outputType` is the
        // definition's own, already walked above.
        v4::ValueBody::Native { .. }
        | v4::ValueBody::External { fallback: None, .. }
        | v4::ValueBody::Incomplete {
            partial_body: None, ..
        } => {}
    }
}

pub fn walk_value<V: V4Visitor + ?Sized>(
    visitor: &mut V,
    cursor: &mut IrCursor,
    value: &v4::Value,
) {
    match value {
        v4::Value::Apply(_, function, argument) => {
            visitor.visit_value(cursor, function);
            visitor.visit_value(cursor, argument);
        }
        v4::Value::Destructure(_, pattern, value, body) => {
            visitor.visit_pattern(cursor, pattern);
            visitor.visit_value(cursor, value);
            visitor.visit_value(cursor, body);
        }
        v4::Value::Field(_, record, _) => visitor.visit_value(cursor, record),
        v4::Value::IfThenElse(_, condition, then_value, else_value) => {
            visitor.visit_value(cursor, condition);
            visitor.visit_value(cursor, then_value);
            visitor.visit_value(cursor, else_value);
        }
        v4::Value::Lambda(_, pattern, body) => {
            visitor.visit_pattern(cursor, pattern);
            visitor.visit_value(cursor, body);
        }
        v4::Value::LetDefinition(_, name, definition, body) => {
            cursor.with_segment(CursorSegment::LetBinding(name.to_string()), |cursor| {
                visitor.visit_definition(cursor, definition);
            });
            visitor.visit_value(cursor, body);
        }
        v4::Value::LetRecursion(_, definitions, body) => {
            for binding in definitions {
                cursor.with_segment(
                    CursorSegment::LetBinding(binding.name().to_string()),
                    |cursor| visitor.visit_definition(cursor, binding.definition()),
                );
            }
            visitor.visit_value(cursor, body);
        }
        v4::Value::List(_, values) | v4::Value::Tuple(_, values) => {
            for (index, value) in values.iter().enumerate() {
                cursor.with_segment(CursorSegment::Argument(index), |cursor| {
                    visitor.visit_value(cursor, value);
                });
            }
        }
        v4::Value::PatternMatch(_, subject, cases) => {
            visitor.visit_value(cursor, subject);
            for (index, case) in cases.iter().enumerate() {
                cursor.with_segment(CursorSegment::PatternCase(index), |cursor| {
                    visitor.visit_pattern(cursor, case.pattern());
                    visitor.visit_value(cursor, case.body());
                });
            }
        }
        v4::Value::Record(_, fields) => {
            for field in fields {
                cursor.with_segment(CursorSegment::Field(field.name().to_string()), |cursor| {
                    visitor.visit_value(cursor, field.value());
                });
            }
        }
        v4::Value::UpdateRecord(_, record, fields) => {
            visitor.visit_value(cursor, record);
            for field in fields {
                cursor.with_segment(CursorSegment::Field(field.name().to_string()), |cursor| {
                    visitor.visit_value(cursor, field.value());
                });
            }
        }
        v4::Value::Constructor(_, _)
        | v4::Value::FieldFunction(_, _)
        | v4::Value::Literal(_, _)
        | v4::Value::Unit(_)
        | v4::Value::Variable(_, _)
        | v4::Value::Reference(_, _)
        | v4::Value::Hole(_, _, _) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::v4::{
        Incompleteness, Type, TypeAttributes, Value, ValueAttributes, ValueBody, ValueDefinition,
    };
    use crate::naming::FQName;

    /// Collects every reference a walk reaches, which is how a caller learns what a definition
    /// depends on.
    #[derive(Default)]
    struct References(Vec<String>);

    impl V4Visitor for References {
        fn visit_value(&mut self, cursor: &mut IrCursor, value: &Value) {
            if let Value::Reference(_, target) = value {
                self.0.push(target.to_canonical_string());
            }
            walk_value(self, cursor, value);
        }
    }

    fn references_of(definition: &ValueDefinition) -> Vec<String> {
        let mut visitor = References::default();
        visitor.visit_definition(&mut IrCursor::root(), definition);
        visitor.0
    }

    fn reference(target: &str) -> Value {
        Value::Reference(
            ValueAttributes::default(),
            FQName::from_canonical_string(target).unwrap(),
        )
    }

    fn incomplete(partial_body: Option<Value>) -> ValueDefinition {
        ValueDefinition {
            input_types: Default::default(),
            output_type: Some(Type::unit(TypeAttributes::default())),
            body: ValueBody::Incomplete {
                incompleteness: Incompleteness::Draft,
                partial_body: partial_body.map(Box::new),
            },
        }
    }

    #[test]
    fn a_walk_reaches_a_reference_inside_an_incomplete_bodys_partial_value() {
        // The partial value is an ordinary expression, so a visitor collecting dependencies has
        // to see what it refers to; skipping it would silently drop the reference.
        let definition = incomplete(Some(Value::Apply(
            ValueAttributes::default(),
            Box::new(reference("acme/shop:pricing#round")),
            Box::new(reference("acme/shop:pricing#subtotal")),
        )));
        assert_eq!(
            references_of(&definition),
            vec![
                "acme/shop:pricing#round".to_string(),
                "acme/shop:pricing#subtotal".to_string(),
            ]
        );
    }

    #[test]
    fn an_incomplete_body_that_kept_nothing_has_nothing_to_walk() {
        assert!(references_of(&incomplete(None)).is_empty());
    }
}

//! Read-only traversal for concrete Classic IR nodes.

use crate::ir::classic;
use crate::traversal::{CursorSegment, IrCursor};

pub trait ClassicVisitor<TA, VA> {
    fn visit_type(&mut self, cursor: &mut IrCursor, value: &classic::Type<TA>) {
        walk_type(self, cursor, value);
    }

    fn visit_pattern(&mut self, cursor: &mut IrCursor, value: &classic::Pattern<VA>) {
        walk_pattern(self, cursor, value);
    }

    fn visit_value(&mut self, cursor: &mut IrCursor, value: &classic::Value<TA, VA>) {
        walk_value(self, cursor, value);
    }

    fn visit_definition(&mut self, cursor: &mut IrCursor, value: &classic::Definition<TA, VA>) {
        walk_definition(self, cursor, value);
    }
}

pub fn walk_type<TA, VA, V>(visitor: &mut V, cursor: &mut IrCursor, value: &classic::Type<TA>)
where
    V: ClassicVisitor<TA, VA> + ?Sized,
{
    match value {
        classic::Type::ExtensibleRecord(_, _, fields) | classic::Type::Record(_, fields) => {
            for field in fields {
                cursor.with_segment(CursorSegment::Field(field.name.to_string()), |cursor| {
                    visitor.visit_type(cursor, &field.ty);
                });
            }
        }
        classic::Type::Function(_, argument, result) => {
            cursor.with_segment(CursorSegment::Argument(0), |cursor| {
                visitor.visit_type(cursor, argument);
            });
            cursor.with_segment(CursorSegment::Branch("result"), |cursor| {
                visitor.visit_type(cursor, result);
            });
        }
        classic::Type::Reference(_, _, arguments) | classic::Type::Tuple(_, arguments) => {
            for (index, argument) in arguments.iter().enumerate() {
                cursor.with_segment(CursorSegment::Argument(index), |cursor| {
                    visitor.visit_type(cursor, argument);
                });
            }
        }
        classic::Type::Unit(_) | classic::Type::Variable(_, _) => {}
    }
}

pub fn walk_pattern<TA, VA, V>(visitor: &mut V, cursor: &mut IrCursor, value: &classic::Pattern<VA>)
where
    V: ClassicVisitor<TA, VA> + ?Sized,
{
    match value {
        classic::Pattern::As(_, pattern, _) => visitor.visit_pattern(cursor, pattern),
        classic::Pattern::Tuple(_, patterns) | classic::Pattern::Constructor(_, _, patterns) => {
            for (index, pattern) in patterns.iter().enumerate() {
                cursor.with_segment(CursorSegment::Argument(index), |cursor| {
                    visitor.visit_pattern(cursor, pattern);
                });
            }
        }
        classic::Pattern::HeadTail(_, head, tail) => {
            cursor.with_segment(CursorSegment::Branch("head"), |cursor| {
                visitor.visit_pattern(cursor, head);
            });
            cursor.with_segment(CursorSegment::Branch("tail"), |cursor| {
                visitor.visit_pattern(cursor, tail);
            });
        }
        classic::Pattern::Wildcard(_)
        | classic::Pattern::EmptyList(_)
        | classic::Pattern::Literal(_, _)
        | classic::Pattern::Unit(_)
        | classic::Pattern::Variable(_, _) => {}
    }
}

pub fn walk_definition<TA, VA, V>(
    visitor: &mut V,
    cursor: &mut IrCursor,
    value: &classic::Definition<TA, VA>,
) where
    V: ClassicVisitor<TA, VA> + ?Sized,
{
    for (index, input) in value.input_types.iter().enumerate() {
        cursor.with_segment(CursorSegment::Argument(index), |cursor| {
            visitor.visit_type(cursor, &input.ty);
        });
    }
    cursor.with_segment(CursorSegment::Branch("output"), |cursor| {
        visitor.visit_type(cursor, &value.output_type);
    });
    cursor.with_segment(CursorSegment::Branch("body"), |cursor| {
        visitor.visit_value(cursor, &value.body);
    });
}

pub fn walk_value<TA, VA, V>(visitor: &mut V, cursor: &mut IrCursor, value: &classic::Value<TA, VA>)
where
    V: ClassicVisitor<TA, VA> + ?Sized,
{
    match value {
        classic::Value::Apply(_, function, argument) => {
            visitor.visit_value(cursor, function);
            visitor.visit_value(cursor, argument);
        }
        classic::Value::Destructure(_, pattern, value, body) => {
            visitor.visit_pattern(cursor, pattern);
            visitor.visit_value(cursor, value);
            visitor.visit_value(cursor, body);
        }
        classic::Value::Field(_, record, _) => visitor.visit_value(cursor, record),
        classic::Value::IfThenElse(_, condition, then_value, else_value) => {
            cursor.with_segment(CursorSegment::Branch("condition"), |cursor| {
                visitor.visit_value(cursor, condition);
            });
            cursor.with_segment(CursorSegment::Branch("then"), |cursor| {
                visitor.visit_value(cursor, then_value);
            });
            cursor.with_segment(CursorSegment::Branch("else"), |cursor| {
                visitor.visit_value(cursor, else_value);
            });
        }
        classic::Value::Lambda(_, pattern, body) => {
            visitor.visit_pattern(cursor, pattern);
            visitor.visit_value(cursor, body);
        }
        classic::Value::LetDefinition(_, name, definition, body) => {
            cursor.with_segment(CursorSegment::LetBinding(name.to_string()), |cursor| {
                visitor.visit_definition(cursor, definition);
            });
            visitor.visit_value(cursor, body);
        }
        classic::Value::LetRecursion(_, definitions, body) => {
            for (name, definition) in definitions {
                cursor.with_segment(CursorSegment::LetBinding(name.to_string()), |cursor| {
                    visitor.visit_definition(cursor, definition);
                });
            }
            visitor.visit_value(cursor, body);
        }
        classic::Value::List(_, values) | classic::Value::Tuple(_, values) => {
            for (index, value) in values.iter().enumerate() {
                cursor.with_segment(CursorSegment::Argument(index), |cursor| {
                    visitor.visit_value(cursor, value);
                });
            }
        }
        classic::Value::PatternMatch(_, subject, cases) => {
            visitor.visit_value(cursor, subject);
            for (index, (pattern, body)) in cases.iter().enumerate() {
                cursor.with_segment(CursorSegment::PatternCase(index), |cursor| {
                    visitor.visit_pattern(cursor, pattern);
                    visitor.visit_value(cursor, body);
                });
            }
        }
        classic::Value::Record(_, fields) => {
            for (name, value) in fields {
                cursor.with_segment(CursorSegment::Field(name.to_string()), |cursor| {
                    visitor.visit_value(cursor, value);
                });
            }
        }
        classic::Value::Update(_, record, fields) => {
            visitor.visit_value(cursor, record);
            for (name, value) in fields {
                cursor.with_segment(CursorSegment::Field(name.to_string()), |cursor| {
                    visitor.visit_value(cursor, value);
                });
            }
        }
        classic::Value::Constructor(_, _)
        | classic::Value::FieldFunction(_, _)
        | classic::Value::Literal(_, _)
        | classic::Value::Unit(_)
        | classic::Value::Variable(_, _)
        | classic::Value::Reference(_, _) => {}
    }
}

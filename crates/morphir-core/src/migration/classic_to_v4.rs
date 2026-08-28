use indexmap::IndexMap;

use crate::ir::{classic, v4};
use crate::migration::{MigrationDiagnostic, MigrationOptions, MigrationReport};
use crate::naming::{FQName, Name, Path};
use crate::traversal::{CursorSegment, IrCursor};

/// Mutable context shared by recursive typed migration operations.
#[derive(Debug, Clone)]
pub struct MigrationContext {
    pub cursor: IrCursor,
    pub report: MigrationReport,
}

impl MigrationContext {
    pub fn new(options: MigrationOptions) -> Self {
        Self {
            cursor: IrCursor::default(),
            report: MigrationReport::new(options),
        }
    }

    pub fn with_segment<R>(
        &mut self,
        segment: CursorSegment,
        operation: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let parent = self.cursor.clone();
        let mut segments = parent.segments().to_vec();
        segments.push(segment);
        self.cursor = IrCursor::from_segments(segments);
        let result = operation(self);
        self.cursor = parent;
        result
    }
}

impl Default for MigrationContext {
    fn default() -> Self {
        Self::new(MigrationOptions::default())
    }
}

fn migrate_name(name: &classic::Name) -> Name {
    Name {
        words: name
            .words
            .iter()
            .map(|word| crate::resolve(*word).to_owned())
            .collect(),
    }
}

pub(crate) fn migrate_path(path: &classic::Path) -> Path {
    Path {
        segments: path.segments.iter().map(migrate_name).collect(),
    }
}

fn migrate_fqname(name: &classic::FQName) -> FQName {
    FQName::new(
        migrate_path(&name.package_path),
        migrate_path(&name.module_path),
        migrate_name(&name.local_name),
    )
}

fn type_attributes(_attributes: &classic::Attrs) -> v4::TypeAttributes {
    v4::TypeAttributes::default()
}

fn value_attributes(
    inferred_type: &classic::Type<classic::Attrs>,
    context: &mut MigrationContext,
) -> Result<v4::ValueAttributes, MigrationDiagnostic> {
    Ok(v4::ValueAttributes {
        source: None,
        inferred_type: Some(Box::new(migrate_type(inferred_type, context)?)),
        extensions: serde_json::Value::Null,
    })
}

pub fn migrate_literal(value: &classic::Literal) -> v4::Literal {
    match value {
        classic::Literal::Bool(value) => v4::Literal::Bool(*value),
        classic::Literal::Char(value) => v4::Literal::Char(*value),
        classic::Literal::String(value) => v4::Literal::String(value.clone()),
        classic::Literal::WholeNumber(value) => v4::Literal::Integer(*value),
        classic::Literal::Float(value) => v4::Literal::Float(*value),
    }
}

pub fn migrate_type(
    value: &classic::Type<classic::Attrs>,
    context: &mut MigrationContext,
) -> Result<v4::Type, MigrationDiagnostic> {
    match value {
        classic::Type::ExtensibleRecord(attributes, name, fields) => {
            let mut migrated = Vec::with_capacity(fields.len());
            for field in fields {
                let field_type = context
                    .with_segment(CursorSegment::Field(field.name.to_string()), |context| {
                        migrate_type(&field.ty, context)
                    })?;
                migrated.push(v4::Field::new(migrate_name(&field.name), field_type));
            }
            Ok(v4::Type::ExtensibleRecord(
                type_attributes(attributes),
                migrate_name(name),
                migrated,
            ))
        }
        classic::Type::Function(attributes, argument, result) => Ok(v4::Type::Function(
            type_attributes(attributes),
            Box::new(migrate_type(argument, context)?),
            Box::new(migrate_type(result, context)?),
        )),
        classic::Type::Record(attributes, fields) => {
            let mut migrated = Vec::with_capacity(fields.len());
            for field in fields {
                let field_type = migrate_type(&field.ty, context)?;
                migrated.push(v4::Field::new(migrate_name(&field.name), field_type));
            }
            Ok(v4::Type::Record(type_attributes(attributes), migrated))
        }
        classic::Type::Reference(attributes, name, arguments) => Ok(v4::Type::Reference(
            type_attributes(attributes),
            migrate_fqname(name),
            arguments
                .iter()
                .map(|argument| migrate_type(argument, context))
                .collect::<Result<_, _>>()?,
        )),
        classic::Type::Tuple(attributes, elements) => Ok(v4::Type::Tuple(
            type_attributes(attributes),
            elements
                .iter()
                .map(|element| migrate_type(element, context))
                .collect::<Result<_, _>>()?,
        )),
        classic::Type::Unit(attributes) => Ok(v4::Type::Unit(type_attributes(attributes))),
        classic::Type::Variable(attributes, name) => Ok(v4::Type::Variable(
            type_attributes(attributes),
            migrate_name(name),
        )),
    }
}

pub fn migrate_pattern(
    value: &classic::Pattern<classic::Type<classic::Attrs>>,
    context: &mut MigrationContext,
) -> Result<v4::Pattern, MigrationDiagnostic> {
    match value {
        classic::Pattern::Wildcard(attributes) => Ok(v4::Pattern::WildcardPattern(
            value_attributes(attributes, context)?,
        )),
        classic::Pattern::As(attributes, pattern, name) => Ok(v4::Pattern::AsPattern(
            value_attributes(attributes, context)?,
            Box::new(migrate_pattern(pattern, context)?),
            migrate_name(name),
        )),
        classic::Pattern::Tuple(attributes, patterns) => Ok(v4::Pattern::TuplePattern(
            value_attributes(attributes, context)?,
            patterns
                .iter()
                .map(|pattern| migrate_pattern(pattern, context))
                .collect::<Result<_, _>>()?,
        )),
        classic::Pattern::Constructor(attributes, name, arguments) => {
            Ok(v4::Pattern::ConstructorPattern(
                value_attributes(attributes, context)?,
                migrate_fqname(name),
                arguments
                    .iter()
                    .map(|argument| migrate_pattern(argument, context))
                    .collect::<Result<_, _>>()?,
            ))
        }
        classic::Pattern::EmptyList(attributes) => Ok(v4::Pattern::EmptyListPattern(
            value_attributes(attributes, context)?,
        )),
        classic::Pattern::HeadTail(attributes, head, tail) => Ok(v4::Pattern::HeadTailPattern(
            value_attributes(attributes, context)?,
            Box::new(migrate_pattern(head, context)?),
            Box::new(migrate_pattern(tail, context)?),
        )),
        classic::Pattern::Literal(attributes, literal) => Ok(v4::Pattern::LiteralPattern(
            value_attributes(attributes, context)?,
            migrate_literal(literal),
        )),
        classic::Pattern::Unit(attributes) => Ok(v4::Pattern::UnitPattern(value_attributes(
            attributes, context,
        )?)),
        classic::Pattern::Variable(attributes, name) => Ok(v4::Pattern::AsPattern(
            value_attributes(attributes, context)?,
            Box::new(v4::Pattern::WildcardPattern(value_attributes(
                attributes, context,
            )?)),
            migrate_name(name),
        )),
    }
}

pub fn migrate_definition(
    definition: &classic::Definition<classic::Attrs, classic::Type<classic::Attrs>>,
    context: &mut MigrationContext,
) -> Result<v4::ValueDefinition, MigrationDiagnostic> {
    migrate_value_definition_parts(
        &definition.input_types,
        &definition.output_type,
        &definition.body,
        context,
    )
}

pub fn migrate_value_definition(
    definition: &classic::ValueDefinition<classic::Attrs, classic::Type<classic::Attrs>>,
    context: &mut MigrationContext,
) -> Result<v4::ValueDefinition, MigrationDiagnostic> {
    migrate_value_definition_parts(
        &definition.input_types,
        &definition.output_type,
        &definition.body,
        context,
    )
}

fn migrate_value_definition_parts(
    input_types: &[classic::value::ValueArgument<classic::Attrs, classic::Type<classic::Attrs>>],
    output_type: &classic::Type<classic::Attrs>,
    body: &classic::Value<classic::Attrs, classic::Type<classic::Attrs>>,
    context: &mut MigrationContext,
) -> Result<v4::ValueDefinition, MigrationDiagnostic> {
    let mut inputs = IndexMap::with_capacity(input_types.len());
    for input in input_types {
        inputs.insert(
            migrate_name(&input.name).to_canonical_string(),
            v4::InputTypeEntry {
                type_attributes: Some(value_attributes(&input.annotation, context)?),
                input_type: migrate_type(&input.ty, context)?,
            },
        );
    }
    Ok(v4::ValueDefinition {
        input_types: inputs,
        output_type: Some(migrate_type(output_type, context)?),
        body: v4::ValueBody::Expression(migrate_value(body, context)?),
    })
}

pub fn migrate_value(
    value: &classic::Value<classic::Attrs, classic::Type<classic::Attrs>>,
    context: &mut MigrationContext,
) -> Result<v4::Value, MigrationDiagnostic> {
    let attributes = value_attributes(
        match value {
            classic::Value::Apply(a, ..)
            | classic::Value::Constructor(a, ..)
            | classic::Value::Destructure(a, ..)
            | classic::Value::Field(a, ..)
            | classic::Value::FieldFunction(a, ..)
            | classic::Value::IfThenElse(a, ..)
            | classic::Value::Lambda(a, ..)
            | classic::Value::LetDefinition(a, ..)
            | classic::Value::LetRecursion(a, ..)
            | classic::Value::List(a, ..)
            | classic::Value::Literal(a, ..)
            | classic::Value::PatternMatch(a, ..)
            | classic::Value::Record(a, ..)
            | classic::Value::Tuple(a, ..)
            | classic::Value::Unit(a)
            | classic::Value::Update(a, ..)
            | classic::Value::Variable(a, ..)
            | classic::Value::Reference(a, ..) => a,
        },
        context,
    )?;

    Ok(match value {
        classic::Value::Apply(_, function, argument) => v4::Value::Apply(
            attributes,
            Box::new(migrate_value(function, context)?),
            Box::new(migrate_value(argument, context)?),
        ),
        classic::Value::Constructor(_, name) => {
            v4::Value::Constructor(attributes, migrate_fqname(name))
        }
        classic::Value::Destructure(_, pattern, value, body) => v4::Value::Destructure(
            attributes,
            migrate_pattern(pattern, context)?,
            Box::new(migrate_value(value, context)?),
            Box::new(migrate_value(body, context)?),
        ),
        classic::Value::Field(_, record, name) => v4::Value::Field(
            attributes,
            Box::new(migrate_value(record, context)?),
            migrate_name(name),
        ),
        classic::Value::FieldFunction(_, name) => {
            v4::Value::FieldFunction(attributes, migrate_name(name))
        }
        classic::Value::IfThenElse(_, condition, then_value, else_value) => v4::Value::IfThenElse(
            attributes,
            Box::new(migrate_value(condition, context)?),
            Box::new(migrate_value(then_value, context)?),
            Box::new(migrate_value(else_value, context)?),
        ),
        classic::Value::Lambda(_, pattern, body) => v4::Value::Lambda(
            attributes,
            migrate_pattern(pattern, context)?,
            Box::new(migrate_value(body, context)?),
        ),
        classic::Value::LetDefinition(_, name, definition, body) => v4::Value::LetDefinition(
            attributes,
            migrate_name(name),
            Box::new(migrate_definition(definition, context)?),
            Box::new(migrate_value(body, context)?),
        ),
        classic::Value::LetRecursion(_, definitions, body) => v4::Value::LetRecursion(
            attributes,
            definitions
                .iter()
                .map(|(name, definition)| {
                    Ok(v4::LetBinding::new(
                        migrate_name(name),
                        migrate_definition(definition, context)?,
                    ))
                })
                .collect::<Result<_, MigrationDiagnostic>>()?,
            Box::new(migrate_value(body, context)?),
        ),
        classic::Value::List(_, values) => v4::Value::List(
            attributes,
            values
                .iter()
                .map(|value| migrate_value(value, context))
                .collect::<Result<_, _>>()?,
        ),
        classic::Value::Literal(_, literal) => {
            v4::Value::Literal(attributes, migrate_literal(literal))
        }
        classic::Value::PatternMatch(_, subject, cases) => v4::Value::PatternMatch(
            attributes,
            Box::new(migrate_value(subject, context)?),
            cases
                .iter()
                .map(|(pattern, body)| {
                    Ok(v4::PatternCase::new(
                        migrate_pattern(pattern, context)?,
                        migrate_value(body, context)?,
                    ))
                })
                .collect::<Result<_, MigrationDiagnostic>>()?,
        ),
        classic::Value::Record(_, fields) => v4::Value::Record(
            attributes,
            fields
                .iter()
                .map(|(name, value)| {
                    Ok(v4::RecordFieldEntry::new(
                        migrate_name(name),
                        migrate_value(value, context)?,
                    ))
                })
                .collect::<Result<_, MigrationDiagnostic>>()?,
        ),
        classic::Value::Tuple(_, values) => v4::Value::Tuple(
            attributes,
            values
                .iter()
                .map(|value| migrate_value(value, context))
                .collect::<Result<_, _>>()?,
        ),
        classic::Value::Unit(_) => v4::Value::Unit(attributes),
        classic::Value::Update(_, record, fields) => v4::Value::UpdateRecord(
            attributes,
            Box::new(migrate_value(record, context)?),
            fields
                .iter()
                .map(|(name, value)| {
                    Ok(v4::RecordFieldEntry::new(
                        migrate_name(name),
                        migrate_value(value, context)?,
                    ))
                })
                .collect::<Result<_, MigrationDiagnostic>>()?,
        ),
        classic::Value::Variable(_, name) => v4::Value::Variable(attributes, migrate_name(name)),
        classic::Value::Reference(_, name) => {
            v4::Value::Reference(attributes, migrate_fqname(name))
        }
    })
}

use indexmap::IndexMap;
use num_bigint::BigInt;

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

/// The diagnostic code a Classic name with no spelling is refused under.
pub const EMPTY_NAME: &str = "empty-name";

/// Migrates one Classic name, refusing a Classic name that has no v4 spelling.
///
/// A v4 name is one or more non-empty segments, and every v4 reader refuses the empty string
/// where a name belongs. A Classic name with no words — or with nothing but empty words — has
/// nothing to encode, so it is refused here rather than written out as a `""` that would produce
/// a v4 document no reader accepts. `cursor` is where the refusal is reported.
pub fn migrate_name(name: &classic::Name, cursor: &IrCursor) -> Result<Name, MigrationDiagnostic> {
    // Classic words are letter-fragmented for acronyms; `from_words` collapses a
    // run of two or more single-letter words back into one initialism.
    let migrated = Name::from_words(
        name.words
            .iter()
            .map(|word| crate::resolve(*word).to_owned()),
    );
    if migrated.is_empty() || migrated.segments().iter().any(|s| s.text().is_empty()) {
        return Err(MigrationDiagnostic::error(
            EMPTY_NAME,
            cursor.clone(),
            "a Classic name with no words has no v4 spelling",
        )
        .with_help("give the name at least one non-empty word in the Classic document"));
    }
    Ok(migrated)
}

#[derive(Debug, Clone)]
pub struct Migrated<T> {
    pub value: T,
    pub report: MigrationReport,
}

pub fn migrate_path(path: &classic::Path, cursor: &IrCursor) -> Result<Path, MigrationDiagnostic> {
    Ok(Path {
        segments: path
            .segments
            .iter()
            .map(|segment| migrate_name(segment, cursor))
            .collect::<Result<_, _>>()?,
    })
}

pub fn migrate_fqname(
    name: &classic::FQName,
    cursor: &IrCursor,
) -> Result<FQName, MigrationDiagnostic> {
    Ok(FQName::new(
        migrate_path(&name.package_path, cursor)?,
        migrate_path(&name.module_path, cursor)?,
        migrate_name(&name.local_name, cursor)?,
    ))
}

fn type_attributes(_attributes: &classic::Attrs) -> v4::TypeAttributes {
    v4::TypeAttributes::default()
}

/// The classic value attribute a v4 [`v4::ValueAttributes`] is built from.
///
/// A morphir-elm value carries its inferred type in the value attribute once type inference has
/// run, and an empty attribute (`{}`) before it has — the spelling the Morphir Compatibility
/// Kit's `versions-0001` case uses. Both are legal classic documents, so the migration is
/// generic over the attribute rather than fixed to the typed one.
pub trait ValueAnnotation {
    fn to_value_attributes(
        &self,
        context: &mut MigrationContext,
    ) -> Result<v4::ValueAttributes, MigrationDiagnostic>;
}

impl ValueAnnotation for classic::Type<classic::Attrs> {
    fn to_value_attributes(
        &self,
        context: &mut MigrationContext,
    ) -> Result<v4::ValueAttributes, MigrationDiagnostic> {
        Ok(v4::ValueAttributes {
            source: None,
            inferred_type: Some(Box::new(migrate_type(self, context)?)),
            extensions: serde_json::Map::new(),
        })
    }
}

/// An untyped classic attribute carries nothing a v4 node keeps.
impl ValueAnnotation for () {
    fn to_value_attributes(
        &self,
        _context: &mut MigrationContext,
    ) -> Result<v4::ValueAttributes, MigrationDiagnostic> {
        Ok(v4::ValueAttributes::default())
    }
}

/// `{}` is empty attributes; anything else is whatever `A` makes of it.
impl<A: ValueAnnotation> ValueAnnotation for classic::Attrs<A> {
    fn to_value_attributes(
        &self,
        context: &mut MigrationContext,
    ) -> Result<v4::ValueAttributes, MigrationDiagnostic> {
        match self {
            classic::Attrs::None => Ok(v4::ValueAttributes::default()),
            classic::Attrs::Some(annotation) => annotation.to_value_attributes(context),
        }
    }
}

pub fn migrate_literal(value: &classic::Literal) -> v4::Literal {
    match value {
        classic::Literal::Bool(value) => v4::Literal::Bool(*value),
        classic::Literal::Char(value) => v4::Literal::Char(*value),
        classic::Literal::String(value) => v4::Literal::String(value.clone()),
        classic::Literal::WholeNumber(value) => v4::Literal::Integer(BigInt::from(*value)),
        // Classic holds a float as a machine number with no lexeme, so the migrated literal is
        // spelled the shortest way that reads back as the same number.
        classic::Literal::Float(value) => v4::Literal::Float(v4::FloatLiteral::from_f64(*value)),
        classic::Literal::Decimal(value) => v4::Literal::Decimal(value.clone()),
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
                migrated.push(v4::Field::new(
                    migrate_name(&field.name, &context.cursor)?,
                    field_type,
                ));
            }
            Ok(v4::Type::ExtensibleRecord(
                type_attributes(attributes),
                migrate_name(name, &context.cursor)?,
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
                migrated.push(v4::Field::new(
                    migrate_name(&field.name, &context.cursor)?,
                    field_type,
                ));
            }
            Ok(v4::Type::Record(type_attributes(attributes), migrated))
        }
        classic::Type::Reference(attributes, name, arguments) => Ok(v4::Type::Reference(
            type_attributes(attributes),
            migrate_fqname(name, &context.cursor)?,
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
            migrate_name(name, &context.cursor)?,
        )),
    }
}

pub fn migrate_pattern<VA: ValueAnnotation>(
    value: &classic::Pattern<VA>,
    context: &mut MigrationContext,
) -> Result<v4::Pattern, MigrationDiagnostic> {
    match value {
        classic::Pattern::Wildcard(attributes) => Ok(v4::Pattern::WildcardPattern(
            attributes.to_value_attributes(context)?,
        )),
        classic::Pattern::As(attributes, pattern, name) => Ok(v4::Pattern::AsPattern(
            attributes.to_value_attributes(context)?,
            Box::new(migrate_pattern(pattern, context)?),
            migrate_name(name, &context.cursor)?,
        )),
        classic::Pattern::Tuple(attributes, patterns) => Ok(v4::Pattern::TuplePattern(
            attributes.to_value_attributes(context)?,
            patterns
                .iter()
                .map(|pattern| migrate_pattern(pattern, context))
                .collect::<Result<_, _>>()?,
        )),
        classic::Pattern::Constructor(attributes, name, arguments) => {
            Ok(v4::Pattern::ConstructorPattern(
                attributes.to_value_attributes(context)?,
                migrate_fqname(name, &context.cursor)?,
                arguments
                    .iter()
                    .map(|argument| migrate_pattern(argument, context))
                    .collect::<Result<_, _>>()?,
            ))
        }
        classic::Pattern::EmptyList(attributes) => Ok(v4::Pattern::EmptyListPattern(
            attributes.to_value_attributes(context)?,
        )),
        classic::Pattern::HeadTail(attributes, head, tail) => Ok(v4::Pattern::HeadTailPattern(
            attributes.to_value_attributes(context)?,
            Box::new(migrate_pattern(head, context)?),
            Box::new(migrate_pattern(tail, context)?),
        )),
        classic::Pattern::Literal(attributes, literal) => Ok(v4::Pattern::LiteralPattern(
            attributes.to_value_attributes(context)?,
            migrate_literal(literal),
        )),
        classic::Pattern::Unit(attributes) => Ok(v4::Pattern::UnitPattern(
            attributes.to_value_attributes(context)?,
        )),
        classic::Pattern::Variable(attributes, name) => Ok(v4::Pattern::AsPattern(
            attributes.to_value_attributes(context)?,
            Box::new(v4::Pattern::WildcardPattern(
                attributes.to_value_attributes(context)?,
            )),
            migrate_name(name, &context.cursor)?,
        )),
    }
}

pub fn migrate_definition<VA: ValueAnnotation>(
    definition: &classic::Definition<classic::Attrs, VA>,
    context: &mut MigrationContext,
) -> Result<v4::ValueDefinition, MigrationDiagnostic> {
    migrate_value_definition_parts(
        &definition.input_types,
        &definition.output_type,
        &definition.body,
        context,
    )
}

pub fn migrate_value_definition<VA: ValueAnnotation>(
    definition: &classic::ValueDefinition<classic::Attrs, VA>,
    context: &mut MigrationContext,
) -> Result<v4::ValueDefinition, MigrationDiagnostic> {
    migrate_value_definition_parts(
        &definition.input_types,
        &definition.output_type,
        &definition.body,
        context,
    )
}

fn migrate_value_definition_parts<VA: ValueAnnotation>(
    input_types: &[classic::value::ValueArgument<classic::Attrs, VA>],
    output_type: &classic::Type<classic::Attrs>,
    body: &classic::Value<classic::Attrs, VA>,
    context: &mut MigrationContext,
) -> Result<v4::ValueDefinition, MigrationDiagnostic> {
    let mut inputs = IndexMap::with_capacity(input_types.len());
    for input in input_types {
        // The contract gives an input parameter a bare type; the v3 parameter attributes have no
        // v4 home.
        inputs.insert(
            migrate_name(&input.name, &context.cursor)?.to_canonical_string(),
            migrate_type(&input.ty, context)?,
        );
    }
    Ok(v4::ValueDefinition {
        input_types: inputs,
        output_type: Some(migrate_type(output_type, context)?),
        body: v4::ValueBody::Expression(migrate_value(body, context)?),
    })
}

pub fn migrate_value<VA: ValueAnnotation>(
    value: &classic::Value<classic::Attrs, VA>,
    context: &mut MigrationContext,
) -> Result<v4::Value, MigrationDiagnostic> {
    let attributes = {
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
        }
    }
    .to_value_attributes(context)?;

    Ok(match value {
        classic::Value::Apply(_, function, argument) => v4::Value::Apply(
            attributes,
            Box::new(migrate_value(function, context)?),
            Box::new(migrate_value(argument, context)?),
        ),
        classic::Value::Constructor(_, name) => {
            v4::Value::Constructor(attributes, migrate_fqname(name, &context.cursor)?)
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
            migrate_name(name, &context.cursor)?,
        ),
        classic::Value::FieldFunction(_, name) => {
            v4::Value::FieldFunction(attributes, migrate_name(name, &context.cursor)?)
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
            migrate_name(name, &context.cursor)?,
            Box::new(migrate_definition(definition, context)?),
            Box::new(migrate_value(body, context)?),
        ),
        classic::Value::LetRecursion(_, definitions, body) => v4::Value::LetRecursion(
            attributes,
            definitions
                .iter()
                .map(|(name, definition)| {
                    Ok(v4::LetBinding::new(
                        migrate_name(name, &context.cursor)?,
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
                        migrate_name(name, &context.cursor)?,
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
                        migrate_name(name, &context.cursor)?,
                        migrate_value(value, context)?,
                    ))
                })
                .collect::<Result<_, MigrationDiagnostic>>()?,
        ),
        classic::Value::Variable(_, name) => {
            v4::Value::Variable(attributes, migrate_name(name, &context.cursor)?)
        }
        classic::Value::Reference(_, name) => {
            v4::Value::Reference(attributes, migrate_fqname(name, &context.cursor)?)
        }
    })
}

pub fn migrate_access(access: &classic::Access) -> v4::Access {
    match access {
        classic::Access::Public => v4::Access::Public,
        classic::Access::Private => v4::Access::Private,
    }
}

fn documentation(value: &str) -> v4::Documentation {
    v4::Documentation::new(value)
}

fn migrate_type_definition(
    definition: &classic::TypeDefinition<classic::Attrs>,
    context: &mut MigrationContext,
) -> Result<v4::TypeDefinition, MigrationDiagnostic> {
    Ok(match definition {
        classic::TypeDefinition::Alias(parameters, body) => {
            v4::TypeDefinition::TypeAliasDefinition {
                type_params: parameters
                    .iter()
                    .map(|parameter| migrate_name(parameter, &context.cursor))
                    .collect::<Result<_, _>>()?,
                type_expr: migrate_type(body, context)?,
            }
        }
        classic::TypeDefinition::Custom(parameters, constructors) => {
            let mut migrated = Vec::with_capacity(constructors.value.len());
            for constructor in &constructors.value {
                migrated.push(v4::ConstructorDefinition {
                    name: migrate_name(&constructor.name, &context.cursor)?,
                    args: constructor
                        .args
                        .iter()
                        .map(|(name, argument)| {
                            Ok(v4::ConstructorArg {
                                name: migrate_name(name, &context.cursor)?,
                                arg_type: migrate_type(argument, context)?,
                            })
                        })
                        .collect::<Result<_, MigrationDiagnostic>>()?,
                });
            }
            v4::TypeDefinition::CustomTypeDefinition {
                type_params: parameters
                    .iter()
                    .map(|parameter| migrate_name(parameter, &context.cursor))
                    .collect::<Result<_, _>>()?,
                constructors: v4::AccessControlled {
                    access: migrate_access(&constructors.access),
                    value: migrated,
                },
            }
        }
    })
}

fn migrate_type_specification(
    specification: &classic::TypeSpecification<classic::Attrs>,
    context: &mut MigrationContext,
) -> Result<v4::TypeSpecification, MigrationDiagnostic> {
    Ok(match specification {
        classic::TypeSpecification::Alias(parameters, body) => {
            v4::TypeSpecification::TypeAliasSpecification {
                // Classic has no annotation vocabulary, so a migrated specification has none.
                annotations: Vec::new(),
                type_params: parameters
                    .iter()
                    .map(|parameter| migrate_name(parameter, &context.cursor))
                    .collect::<Result<_, _>>()?,
                type_expr: migrate_type(body, context)?,
            }
        }
        classic::TypeSpecification::Opaque(parameters) => {
            v4::TypeSpecification::OpaqueTypeSpecification {
                annotations: Vec::new(),
                type_params: parameters
                    .iter()
                    .map(|parameter| migrate_name(parameter, &context.cursor))
                    .collect::<Result<_, _>>()?,
            }
        }
        classic::TypeSpecification::Custom(parameters, constructors) => {
            v4::TypeSpecification::CustomTypeSpecification {
                annotations: Vec::new(),
                type_params: parameters
                    .iter()
                    .map(|parameter| migrate_name(parameter, &context.cursor))
                    .collect::<Result<_, _>>()?,
                constructors: constructors
                    .iter()
                    .map(|constructor| {
                        Ok(v4::ConstructorSpecification {
                            name: migrate_name(&constructor.name, &context.cursor)?,
                            args: constructor
                                .args
                                .iter()
                                .map(|(name, argument)| {
                                    Ok(v4::ConstructorArgSpec {
                                        name: migrate_name(name, &context.cursor)?,
                                        arg_type: migrate_type(argument, context)?,
                                    })
                                })
                                .collect::<Result<_, MigrationDiagnostic>>()?,
                        })
                    })
                    .collect::<Result<_, MigrationDiagnostic>>()?,
            }
        }
    })
}

fn migrate_value_specification(
    specification: &classic::ValueSpecification<classic::Attrs>,
    context: &mut MigrationContext,
) -> Result<v4::ValueSpecification, MigrationDiagnostic> {
    Ok(v4::ValueSpecification {
        annotations: Vec::new(),
        inputs: specification
            .inputs
            .iter()
            .map(|parameter| {
                Ok((
                    migrate_name(&parameter.name, &context.cursor)?.to_canonical_string(),
                    migrate_type(&parameter.ty, context)?,
                ))
            })
            .collect::<Result<_, MigrationDiagnostic>>()?,
        output: migrate_type(&specification.output, context)?,
    })
}

fn migrate_module_specification(
    specification: &classic::ModuleSpecification<classic::Attrs>,
    context: &mut MigrationContext,
) -> Result<v4::ModuleSpecification, MigrationDiagnostic> {
    let types = specification
        .types
        .iter()
        .map(|(name, documented)| {
            Ok((
                migrate_name(name, &context.cursor)?.to_canonical_string(),
                v4::Documented::new(
                    Some(documentation(&documented.doc)),
                    migrate_type_specification(&documented.value, context)?,
                ),
            ))
        })
        .collect::<Result<_, MigrationDiagnostic>>()?;
    let values = specification
        .values
        .iter()
        .map(|(name, documented)| {
            Ok((
                migrate_name(name, &context.cursor)?.to_canonical_string(),
                v4::Documented::new(
                    Some(documentation(&documented.doc)),
                    migrate_value_specification(&documented.value, context)?,
                ),
            ))
        })
        .collect::<Result<_, MigrationDiagnostic>>()?;
    Ok(v4::ModuleSpecification {
        annotations: Vec::new(),
        types,
        values,
        doc: specification.doc.as_deref().map(documentation),
    })
}

pub fn migrate_package_specification(
    specification: &classic::PackageSpecification<classic::Attrs>,
    context: &mut MigrationContext,
) -> Result<v4::PackageSpecification, MigrationDiagnostic> {
    Ok(v4::PackageSpecification {
        modules: specification
            .modules
            .iter()
            .map(|entry| {
                Ok((
                    migrate_path(&entry.path, &context.cursor)?.to_canonical_string(),
                    migrate_module_specification(&entry.specification, context)?,
                ))
            })
            .collect::<Result<_, MigrationDiagnostic>>()?,
    })
}

pub fn migrate_module_definition<VA: ValueAnnotation>(
    definition: &classic::ModuleDefinition<classic::Attrs, VA>,
    context: &mut MigrationContext,
) -> Result<v4::ModuleDefinition, MigrationDiagnostic> {
    let types = definition
        .types
        .iter()
        .map(|(name, controlled)| {
            Ok((
                migrate_name(name, &context.cursor)?.to_canonical_string(),
                v4::AccessControlled {
                    access: migrate_access(&controlled.access),
                    value: v4::Documented::new(
                        Some(documentation(&controlled.value.doc)),
                        migrate_type_definition(&controlled.value.value, context)?,
                    ),
                },
            ))
        })
        .collect::<Result<_, MigrationDiagnostic>>()?;
    let values = definition
        .values
        .iter()
        .map(|(name, controlled)| {
            Ok((
                migrate_name(name, &context.cursor)?.to_canonical_string(),
                v4::AccessControlled {
                    access: migrate_access(&controlled.access),
                    value: v4::Documented::new(
                        Some(documentation(&controlled.value.doc)),
                        migrate_value_definition(&controlled.value.value, context)?,
                    ),
                },
            ))
        })
        .collect::<Result<_, MigrationDiagnostic>>()?;
    Ok(v4::ModuleDefinition {
        types,
        values,
        doc: definition.doc.as_deref().map(documentation),
    })
}

pub fn migrate_distribution(
    distribution: &classic::Distribution,
    options: MigrationOptions,
) -> Result<Migrated<v4::IRFile>, MigrationDiagnostic> {
    let mut context = MigrationContext::new(options);
    if distribution.format_version != 3 {
        return Err(MigrationDiagnostic::error(
            "unsupported-source-version",
            context.cursor.clone(),
            format!(
                "typed Classic migration requires formatVersion 3, found {}",
                distribution.format_version
            ),
        ));
    }

    let classic::DistributionBody::Library(package_name, dependencies, package) =
        &distribution.distribution;

    let dependencies = dependencies
        .iter()
        .map(|(name, specification)| {
            Ok((
                migrate_path(name, &context.cursor)?.to_canonical_string(),
                migrate_package_specification(specification, &mut context)?,
            ))
        })
        .collect::<Result<_, MigrationDiagnostic>>()?;
    let modules = package
        .modules
        .iter()
        .map(|entry| {
            Ok((
                migrate_path(&entry.path, &context.cursor)?.to_canonical_string(),
                v4::AccessControlled {
                    access: migrate_access(&entry.definition.access),
                    value: migrate_module_definition(&entry.definition.value, &mut context)?,
                },
            ))
        })
        .collect::<Result<_, MigrationDiagnostic>>()?;

    Ok(Migrated {
        value: v4::IRFile {
            format_version: v4::FormatVersion::Integer(4),
            distribution: v4::Distribution::Library(v4::LibraryContent {
                package_name: crate::naming::PackageName::new(migrate_path(
                    package_name,
                    &context.cursor,
                )?),
                dependencies,
                def: v4::PackageDefinition { modules },
            }),
        },
        report: context.report,
    })
}

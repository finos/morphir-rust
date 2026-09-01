//! Mapping from Morphir type forms onto the dialect-neutral schema model.

use std::collections::{BTreeMap, BTreeSet};

use morphir_projection::{Constructor, NamedType, TypeDeclaration, TypeExpr};

use super::names::{field_name, schema_name, variant_name};
use super::{Schema, SchemaField, SchemaVariant};
use crate::SchemaDiagnostic;

const SDK_BOOL: &str = "morphir/SDK:basics#bool";
const SDK_INT: &str = "morphir/SDK:basics#int";
const SDK_FLOAT: &str = "morphir/SDK:basics#float";
const SDK_STRING: &str = "morphir/SDK:string#string";
const SDK_CHAR: &str = "morphir/SDK:char#char";
const SDK_MAYBE: &str = "morphir/SDK:maybe#maybe";
const SDK_LIST: &str = "morphir/SDK:list#list";
const SDK_SET: &str = "morphir/SDK:set#set";
const SDK_DICT: &str = "morphir/SDK:dict#dict";
const SDK_RESULT: &str = "morphir/SDK:result#result";

/// The property name that tells one custom-type variant from another.
pub(super) const DISCRIMINATOR: &str = "kind";

/// Everything the type mapping needs to resolve a reference.
pub(super) struct Context<'a> {
    /// Every declaration visible to this projection, keyed by canonical FQName.
    pub(super) declared: &'a BTreeMap<String, TypeDeclaration>,
}

/// Project one Morphir declaration into a schema.
///
/// Canonical FQNames of the declarations this schema refers to are added to
/// `referenced`, so the caller can project them in turn.
pub(super) fn project_declaration(
    context: &Context<'_>,
    declaration: &TypeDeclaration,
    referenced: &mut BTreeSet<String>,
) -> Result<Schema, SchemaDiagnostic> {
    let source_name = declaration.source_name();
    match declaration {
        TypeDeclaration::Alias { value, .. } => {
            project_type(context, source_name, value, referenced)
        }
        TypeDeclaration::Custom { constructors, .. } => {
            project_custom(context, source_name, constructors, referenced)
        }
        TypeDeclaration::Opaque { .. } => Err(SchemaDiagnostic::unsupported_form(
            source_name,
            "an opaque type has no visible representation to project",
        )),
        TypeDeclaration::Incomplete { incompleteness, .. } => {
            Err(SchemaDiagnostic::unsupported_form(
                source_name,
                format!("an incomplete declaration ({incompleteness:?}) has no schema"),
            ))
        }
    }
}

/// Project a custom type as an enumeration or as a discriminated choice.
///
/// A discriminated choice writes [`DISCRIMINATOR`] into every variant's
/// `properties` and `required`. `kind` is an ordinary Morphir identifier, so
/// a constructor is free to declare an argument of that name — and if one
/// does, the rendered variant cannot carry both: the discriminator would
/// overwrite the real field's schema and its name would appear twice in
/// `required`, which JSON Schema 2020-12 and the OpenAPI 3.0 metaschema both
/// forbid. The rule is therefore that a constructor argument may not be named
/// `kind`: the collision is rejected here, as a `JSC003` unsupported form
/// naming the constructor and the argument, rather than silently corrupting
/// the schema. Like every other `JSC003`, `unsupported: "warn-and-skip"`
/// downgrades it to a warning that omits just this declaration.
fn project_custom(
    context: &Context<'_>,
    source_name: &str,
    constructors: &[Constructor],
    referenced: &mut BTreeSet<String>,
) -> Result<Schema, SchemaDiagnostic> {
    if constructors
        .iter()
        .all(|constructor| constructor.arguments.is_empty())
    {
        let mut values = constructors
            .iter()
            .map(|constructor| variant_name(&constructor.name))
            .collect::<Vec<_>>();
        values.sort();
        return Ok(Schema::Enumeration(values));
    }
    let variants = constructors
        .iter()
        .map(|constructor| {
            let schema = project_object(context, source_name, &constructor.arguments, referenced)?;
            reject_discriminator_collision(source_name, constructor, &schema)?;
            Ok(SchemaVariant {
                name: variant_name(&constructor.name),
                schema,
                source_name: constructor.source_name.clone(),
            })
        })
        .collect::<Result<Vec<_>, SchemaDiagnostic>>()?;
    Ok(Schema::OneOf {
        discriminator: DISCRIMINATOR.to_owned(),
        variants,
    })
}

/// Reject a constructor argument whose projected property name is
/// [`DISCRIMINATOR`].
///
/// The name is compared after `field_name` normalization, because that is the
/// name the rendered variant would carry: a Morphir argument named `Kind` and
/// one named `kind` both project to the property `kind` and both collide.
fn reject_discriminator_collision(
    source_name: &str,
    constructor: &Constructor,
    schema: &Schema,
) -> Result<(), SchemaDiagnostic> {
    let Schema::Object { fields, .. } = schema else {
        return Ok(());
    };
    if !fields.iter().any(|field| field.name == DISCRIMINATOR) {
        return Ok(());
    }
    Err(SchemaDiagnostic::unsupported_form(
        source_name,
        format!(
            "constructor '{}' declares an argument that projects to the property '{DISCRIMINATOR}', \
             which is the discriminator this backend writes into every variant of a custom type; \
             rename the argument, because one property cannot be both",
            constructor.name
        ),
    ))
}

/// Project a Morphir type expression into a schema.
pub(super) fn project_type(
    context: &Context<'_>,
    owner_source: &str,
    tpe: &TypeExpr,
    referenced: &mut BTreeSet<String>,
) -> Result<Schema, SchemaDiagnostic> {
    match tpe {
        TypeExpr::Unit => Ok(Schema::Null),
        TypeExpr::Tuple(elements) => elements
            .iter()
            .map(|element| project_type(context, owner_source, element, referenced))
            .collect::<Result<Vec<_>, _>>()
            .map(Schema::Tuple),
        TypeExpr::Record(fields) => project_object(context, owner_source, fields, referenced),
        TypeExpr::Reference {
            source_name,
            arguments,
        } => project_reference(context, owner_source, source_name, arguments, referenced),
        TypeExpr::Variable(parameter) => Err(SchemaDiagnostic::unsupported_form(
            owner_source,
            format!("type parameter '{parameter}' is unbound, so it has no schema"),
        )),
        TypeExpr::ExtensibleRecord { variable, .. } => Err(SchemaDiagnostic::unsupported_form(
            owner_source,
            format!("an extensible record over row variable '{variable}' has no closed schema"),
        )),
        TypeExpr::Function { .. } => Err(SchemaDiagnostic::unsupported_form(
            owner_source,
            "a function used as data has no schema",
        )),
    }
}

/// Project named Morphir fields into an object with its required-field list.
fn project_object(
    context: &Context<'_>,
    owner_source: &str,
    source_fields: &[NamedType],
    referenced: &mut BTreeSet<String>,
) -> Result<Schema, SchemaDiagnostic> {
    let fields = source_fields
        .iter()
        .map(|field| {
            Ok(SchemaField {
                name: field_name(&field.name),
                schema: project_type(context, owner_source, &field.tpe, referenced)?,
                // A Morphir record field is always present. Optionality is
                // carried by the field's own type, not by its presence.
                required: true,
                doc: None,
            })
        })
        .collect::<Result<Vec<SchemaField>, SchemaDiagnostic>>()?;
    let required = fields
        .iter()
        .filter(|field| field.required)
        .map(|field| field.name.clone())
        .collect();
    Ok(Schema::Object { fields, required })
}

/// Project a Morphir type reference, resolving SDK types and declared types.
fn project_reference(
    context: &Context<'_>,
    owner_source: &str,
    source_name: &str,
    arguments: &[TypeExpr],
    referenced: &mut BTreeSet<String>,
) -> Result<Schema, SchemaDiagnostic> {
    let argument =
        |index: usize, referenced: &mut BTreeSet<String>| -> Result<Schema, SchemaDiagnostic> {
            project_type(context, owner_source, &arguments[index], referenced)
        };
    match (source_name, arguments.len()) {
        (SDK_BOOL, 0) => Ok(Schema::Boolean),
        (SDK_INT, 0) => Ok(Schema::Integer {
            format: Some("int64"),
        }),
        (SDK_FLOAT, 0) => Ok(Schema::Number {
            format: Some("double"),
        }),
        (SDK_STRING, 0) => Ok(Schema::Text { max_length: None }),
        (SDK_CHAR, 0) => Ok(Schema::Text {
            max_length: Some(1),
        }),
        (SDK_MAYBE, 1) => Ok(Schema::Union(vec![argument(0, referenced)?, Schema::Null])),
        (SDK_LIST, 1) => Ok(Schema::Array {
            items: Box::new(argument(0, referenced)?),
            unique: false,
        }),
        (SDK_SET, 1) => Ok(Schema::Array {
            items: Box::new(argument(0, referenced)?),
            unique: true,
        }),
        (SDK_DICT, 2) => {
            if !is_text_key(&arguments[0]) {
                return Err(SchemaDiagnostic::unsupported_form(
                    owner_source,
                    "a Dict with a non-String key has no object schema",
                ));
            }
            Ok(Schema::Map {
                values: Box::new(argument(1, referenced)?),
            })
        }
        (SDK_RESULT, 2) => project_result(
            context,
            owner_source,
            &arguments[0],
            &arguments[1],
            referenced,
        ),
        _ if context.declared.contains_key(source_name) => {
            referenced.insert(source_name.to_owned());
            Ok(Schema::Reference(schema_name(source_name)))
        }
        _ => Err(SchemaDiagnostic::unsupported_form(
            owner_source,
            format!("no schema projection for '{source_name}'"),
        )),
    }
}

/// Project the SDK's `Result error value` as the whole discriminated choice
/// between its `Err` and `Ok` members.
///
/// Mirrors `morphir-avro-extension`'s `project_result`: each member is
/// wrapped in a single-field object (`error` for `Err`, `value` for `Ok`)
/// rather than being the argument's own schema, so a scalar or reference
/// member still has a home for the `kind` discriminator property. Operation
/// projection reaches for this only when `ResultResponses::Data` keeps the
/// whole shape in one response; `ResultResponses::Split` instead projects
/// `error` and `value` on their own, without ever building this choice.
fn project_result(
    context: &Context<'_>,
    owner_source: &str,
    error: &TypeExpr,
    value: &TypeExpr,
    referenced: &mut BTreeSet<String>,
) -> Result<Schema, SchemaDiagnostic> {
    let error_field = NamedType {
        name: "error".to_owned(),
        tpe: error.clone(),
    };
    let value_field = NamedType {
        name: "value".to_owned(),
        tpe: value.clone(),
    };
    Ok(Schema::OneOf {
        discriminator: DISCRIMINATOR.to_owned(),
        variants: vec![
            SchemaVariant {
                name: variant_name("err"),
                schema: project_object(
                    context,
                    owner_source,
                    std::slice::from_ref(&error_field),
                    referenced,
                )?,
                source_name: "morphir/SDK:result#err".to_owned(),
            },
            SchemaVariant {
                name: variant_name("ok"),
                schema: project_object(
                    context,
                    owner_source,
                    std::slice::from_ref(&value_field),
                    referenced,
                )?,
                source_name: "morphir/SDK:result#ok".to_owned(),
            },
        ],
    })
}

/// Only a `String` key maps onto an object with named properties.
fn is_text_key(tpe: &TypeExpr) -> bool {
    matches!(
        tpe,
        TypeExpr::Reference { source_name, arguments }
            if source_name == SDK_STRING && arguments.is_empty()
    )
}

//! Synthesize OpenAPI operations from Morphir value specifications.
//!
//! [`project_operations`] turns the value specifications
//! [`SchemaOptions::projection`] selects into [`Operation`]s: `Schemas`
//! selects none, `OperationsEntryPoints` selects only declared entry
//! points, and `OperationsPublic` selects every public value specification.
//! Every selection uses the default mapping — `POST`, path
//! `/{module}/{value}`, arguments as a request-body object, the output type
//! as the `200` response — because per-operation overrides are a later plan
//! step.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use morphir_projection::{
    EntryPointMetadata, ProjectionModule, ProjectionPackage, ValueKind, ValueSpecification,
};

use super::names::{field_name, operation_id};
use super::types::{Context, project_type};
use super::{Schema, SchemaField, SchemaProjection, close_definitions, declared_types};
use crate::options::Projection;
use crate::{HttpMethod, SchemaDiagnostic, SchemaOptions, Unsupported};

/// One HTTP operation synthesized from a Morphir value specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    /// Exact canonical Morphir FQName of the projected value.
    pub source_name: String,
    /// HTTP method; always [`HttpMethod::Post`] before any override.
    pub method: HttpMethod,
    /// Request path template.
    pub path: String,
    /// Request-body fields; empty for a [`ValueKind::Constant`], which has
    /// no inputs and so no request body.
    pub request: Vec<SchemaField>,
    /// The projected `200` response schema; [`Schema::Null`] when the value
    /// specification carries no output type.
    pub response: Schema,
    /// Declared entry-point metadata, present only when this operation
    /// projects a declared application entry point.
    pub entry_point: Option<EntryPointMetadata>,
    /// Optional source documentation.
    pub doc: Option<String>,
}

/// Select value specifications per `options.projection` and project each
/// into an [`Operation`], in module and then declaration order.
///
/// A type an operation's request or response reaches that the schema walk
/// from public type roots did not already register is added to
/// `projection.definitions`, so a rendered document never carries a `$ref`
/// outside `components/schemas`.
///
/// A value specification whose signature has no schema is a form like any
/// other Morphir declaration: under [`Unsupported::Error`] (the default) it
/// fails the whole generation; under [`Unsupported::WarnAndSkip`] the
/// operation is omitted from `paths`, a `JSC003` warning is recorded naming
/// its Morphir FQName, and the rest of the document still renders.
///
/// A path claimed by two operations, or an `operationId` claimed by two
/// operations, is a `SchemaDiagnostic::operation_collision` (`OAS001`)
/// naming both Morphir FQNames — a Morphir-source ambiguity, so it is
/// always an error regardless of `options.unsupported`. `render_openapi`
/// cannot fail, so this check happens here, before any document is built.
pub fn project_operations(
    package: &ProjectionPackage,
    projection: &mut SchemaProjection,
    options: &SchemaOptions,
) -> Result<Vec<Operation>, SchemaDiagnostic> {
    if matches!(options.projection, Projection::Schemas) {
        return Ok(Vec::new());
    }

    let declared = declared_types(package);
    let context = Context {
        declared: &declared,
    };

    let mut operations = Vec::new();
    let mut claimed_paths: BTreeMap<String, String> = BTreeMap::new();
    let mut claimed_ids: BTreeMap<String, String> = BTreeMap::new();
    let mut discovered: BTreeSet<String> = BTreeSet::new();
    let mut warnings: Vec<(SchemaDiagnostic, bool)> = Vec::new();

    for module in &package.modules {
        for value in &module.values {
            if !selected(options.projection, value) {
                continue;
            }

            // A fresh set per attempt: only merged into `discovered` once the
            // operation itself succeeds, the same way `close_definitions`
            // only enqueues a declaration's references once that declaration
            // projected successfully. A type reached only by a skipped
            // operation must not be pulled into `components/schemas`.
            let mut referenced = BTreeSet::new();
            let operation = match project_operation(&context, module, value, &mut referenced) {
                Ok(operation) => operation,
                Err(diagnostic) => {
                    if options.unsupported == Unsupported::Error {
                        return Err(diagnostic);
                    }
                    warnings.push((diagnostic, true));
                    continue;
                }
            };
            discovered.extend(referenced);

            let path_key = format!("{:?} {}", operation.method, operation.path);
            if let Some(existing) = claimed_paths.get(&path_key) {
                return Err(SchemaDiagnostic::operation_collision(
                    &operation.source_name,
                    format!(
                        "{:?} {} is claimed by both '{existing}' and '{}'",
                        operation.method, operation.path, operation.source_name
                    ),
                ));
            }
            claimed_paths.insert(path_key, operation.source_name.clone());

            let id = operation_id(&operation.source_name);
            if let Some(existing) = claimed_ids.get(&id) {
                return Err(SchemaDiagnostic::operation_collision(
                    &operation.source_name,
                    format!(
                        "operationId '{id}' is claimed by both '{existing}' and '{}'",
                        operation.source_name
                    ),
                ));
            }
            claimed_ids.insert(id, operation.source_name.clone());

            operations.push(operation);
        }
    }

    extend_definitions(&context, options, projection, discovered)?;
    projection.diagnostics.extend(warnings);

    Ok(operations)
}

/// Whether `options.projection` includes `value` as an operation.
fn selected(projection: Projection, value: &ValueSpecification) -> bool {
    match projection {
        Projection::Schemas => false,
        Projection::OperationsEntryPoints => value.entry_point.is_some(),
        Projection::OperationsPublic => true,
    }
}

/// Project one value specification into an [`Operation`] under the default
/// mapping.
fn project_operation(
    context: &Context<'_>,
    module: &ProjectionModule,
    value: &ValueSpecification,
    discovered: &mut BTreeSet<String>,
) -> Result<Operation, SchemaDiagnostic> {
    let request = if matches!(value.value_kind, ValueKind::Constant) {
        Vec::new()
    } else {
        value
            .inputs
            .iter()
            .map(|input| {
                Ok(SchemaField {
                    name: field_name(&input.name),
                    schema: project_type(context, &value.source_name, &input.tpe, discovered)?,
                    required: true,
                    doc: None,
                })
            })
            .collect::<Result<Vec<_>, SchemaDiagnostic>>()?
    };

    let response = match &value.output {
        Some(output) => project_type(context, &value.source_name, output, discovered)?,
        None => Schema::Null,
    };

    Ok(Operation {
        source_name: value.source_name.clone(),
        method: HttpMethod::Post,
        path: default_path(module, value),
        request,
        response,
        entry_point: value.entry_point.clone(),
        doc: value.doc.clone(),
    })
}

/// The default `/{module}/{entryPoint}` path: module segments lowercased and
/// joined with `/`, and the value name in `lowerCamelCase`.
fn default_path(module: &ProjectionModule, value: &ValueSpecification) -> String {
    let module_segment = module
        .path
        .iter()
        .map(|segment| segment.to_lowercase())
        .collect::<Vec<_>>()
        .join("/");
    format!("/{module_segment}/{}", field_name(&value.name))
}

/// Add every type an operation's request or response reached that is not
/// already a registered definition, via the same [`close_definitions`]
/// closure [`super::project`] uses.
///
/// `claimed` and `visited` are rebuilt from `projection.definitions` rather
/// than started empty, so a dependency type this walk reaches that projects
/// to a schema name already claimed by an unrelated source is caught as the
/// same `JSC004` collision `project` itself would raise, instead of
/// silently overwriting that name's `$ref` target.
fn extend_definitions(
    context: &Context<'_>,
    options: &SchemaOptions,
    projection: &mut SchemaProjection,
    seeds: BTreeSet<String>,
) -> Result<(), SchemaDiagnostic> {
    let mut claimed: BTreeMap<String, String> = projection
        .definitions
        .values()
        .map(|named| (named.name.clone(), named.source_name.clone()))
        .collect();
    let mut visited: BTreeSet<String> = projection
        .definitions
        .values()
        .map(|named| named.source_name.clone())
        .collect();
    let mut queue: VecDeque<String> = VecDeque::new();
    for source_name in seeds {
        if visited.insert(source_name.clone()) {
            queue.push_back(source_name);
        }
    }

    let mut diagnostics = Vec::new();
    close_definitions(
        context,
        options,
        &mut queue,
        &mut visited,
        &mut claimed,
        &mut projection.definitions,
        &mut diagnostics,
    )?;
    projection.diagnostics.extend(diagnostics);
    Ok(())
}

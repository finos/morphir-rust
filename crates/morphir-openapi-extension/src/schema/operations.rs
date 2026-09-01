//! Synthesize OpenAPI operations from Morphir value specifications.
//!
//! [`project_operations`] turns the value specifications
//! [`SchemaOptions::projection`] selects into [`Operation`]s: `Schemas`
//! selects none, `OperationsEntryPoints` selects only declared entry
//! points, and `OperationsPublic` selects every public value specification.
//! Every selection starts from the default mapping — `POST`, path
//! `/{module}/{value}`, arguments as a request-body object, the output type
//! as the `200` response — and then applies `options.operations`, keyed by
//! canonical Morphir FQName, to move the method, the path, and individual
//! parameters. `options.result_responses` additionally decides whether a
//! `Result`-shaped output stays as one `200` body or splits its `Ok` member
//! into `200` and its `Err` member into `options.error_status`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use morphir_projection::{
    EntryPointMetadata, ProjectionModule, ProjectionPackage, TypeExpr, ValueKind,
    ValueSpecification,
};

use super::names::{field_name, operation_id};
use super::types::{Context, project_type};
use super::{
    NamedSchema, Schema, SchemaField, SchemaProjection, close_definitions, declared_types,
    references,
};
use crate::options::{OperationOverride, Projection, ResultResponses};
use crate::{HttpMethod, ParameterBinding, SchemaDiagnostic, SchemaOptions, Unsupported};

/// Canonical FQName of the SDK's `Result` type. `Result` is identified by
/// this exact source name, never by shape, so a package-local type that
/// happens to look like an error/value choice is never mistaken for it.
const SDK_RESULT: &str = "morphir/SDK:result#result";

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
    /// no inputs and so no request body, and also once every input has been
    /// moved into `parameters` by an override.
    pub request: Vec<SchemaField>,
    /// Fields an override moved out of the request body, paired with where
    /// each is bound. Rendered as OpenAPI Parameter Objects rather than
    /// request-body properties.
    pub parameters: Vec<(ParameterBinding, SchemaField)>,
    /// The projected `200` response schema; [`Schema::Null`] when the value
    /// specification carries no output type. Under
    /// [`crate::ResultResponses::Split`] applied to a `Result`-shaped
    /// output, this is the `Ok` member's schema rather than the whole
    /// `Result`.
    pub response: Schema,
    /// The paired status code and `Err` member schema
    /// [`crate::ResultResponses::Split`] adds alongside `response`. `None`
    /// under [`crate::ResultResponses::Data`], and `None` whenever the
    /// output is not `Result`-shaped regardless of `result_responses`.
    pub error_response: Option<(u16, Schema)>,
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
/// its Morphir FQName, and the rest of the document still renders. The same
/// rule applies one hop further out: an operation whose request or response
/// only refers to a type — never touching an unsupported form itself — can
/// still end up pointing at nothing if that type's own closure lost a
/// definition to `Unsupported::WarnAndSkip` (`extend_definitions`'s
/// `close_definitions` call sweeps `projection.definitions`, but an
/// operation is a second source of references into that same namespace,
/// outside `definitions` itself). `drop_dangling_operations` applies the
/// identical rule to that case: the operation is dropped and warned about
/// by its own FQName, so a rendered document can never carry a `$ref` with
/// no `components/schemas` entry behind it, however many hops away the
/// unsupported form was.
///
/// A path claimed by two operations, or an `operationId` claimed by two
/// operations, is a `SchemaDiagnostic::operation_collision` (`OAS001`)
/// naming both Morphir FQNames — a Morphir-source ambiguity, so it is
/// always an error regardless of `options.unsupported`. `render_openapi`
/// cannot fail, so this check happens here, before any document is built.
///
/// An `options.operations` key that names no value specification the
/// package declares is `SchemaDiagnostic::unknown_operation` (`OAS002`),
/// checked up front against every declared value — not only the ones this
/// projection mode selects — and, like `OAS001`, always an error regardless
/// of `options.unsupported`: a misconfigured override is not a Morphir form
/// this backend cannot project, so skipping it silently would hide the
/// mistake rather than the intended behavior.
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

    // Every value specification the package declares, whether or not this
    // projection mode selects it as an operation: `options.operations`'
    // keys are checked against this set, not against the narrower set that
    // actually became an `Operation`, because "no value specification" is
    // what `SchemaDiagnostic::unknown_operation` (`OAS002`) promises.
    let declared_values: BTreeSet<&str> = package
        .modules
        .iter()
        .flat_map(|module| &module.values)
        .map(|value| value.source_name.as_str())
        .collect();
    if let Some(unknown) = options
        .operations
        .keys()
        .find(|source_name| !declared_values.contains(source_name.as_str()))
    {
        return Err(SchemaDiagnostic::unknown_operation(
            unknown,
            format!("'{unknown}' is not a value specification this package declares"),
        ));
    }

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
            let mut operation =
                match project_operation(&context, module, value, options, &mut referenced) {
                    Ok(operation) => operation,
                    Err(diagnostic) => {
                        if options.unsupported == Unsupported::Error {
                            return Err(diagnostic);
                        }
                        warnings.push((diagnostic, true));
                        continue;
                    }
                };
            if let Some(override_) = options.operations.get(&operation.source_name) {
                apply_override(&mut operation, override_)?;
            }
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
    let operations = drop_dangling_operations(operations, &projection.definitions, &mut warnings);
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
/// mapping. The caller applies `options.operations`' override, if any, once
/// this returns.
fn project_operation(
    context: &Context<'_>,
    module: &ProjectionModule,
    value: &ValueSpecification,
    options: &SchemaOptions,
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

    let (response, error_response) = match &value.output {
        Some(output) => project_response(context, value, output, options, discovered)?,
        None => (Schema::Null, None),
    };

    Ok(Operation {
        source_name: value.source_name.clone(),
        method: HttpMethod::Post,
        path: default_path(module, value),
        request,
        parameters: Vec::new(),
        response,
        error_response,
        entry_point: value.entry_point.clone(),
        doc: value.doc.clone(),
    })
}

/// Project a value specification's output type into its `200` response and,
/// under [`ResultResponses::Split`], its paired error response.
///
/// `Result` is detected by `output`'s own source FQName
/// (`morphir/SDK:result#result`), never by shape: a package-local type that
/// happens to carry an `Ok`/`Err`-shaped choice is never split just because
/// it looks like one. Under [`ResultResponses::Data`] — the default — or
/// when `output` is not `Result`-shaped, the whole output type is projected
/// as one `200` response and `error_response` is `None`.
fn project_response(
    context: &Context<'_>,
    value: &ValueSpecification,
    output: &TypeExpr,
    options: &SchemaOptions,
    discovered: &mut BTreeSet<String>,
) -> Result<(Schema, Option<(u16, Schema)>), SchemaDiagnostic> {
    if let (ResultResponses::Split, Some((error, ok))) =
        (options.result_responses, result_arguments(output))
    {
        let ok_schema = project_type(context, &value.source_name, ok, discovered)?;
        let error_schema = project_type(context, &value.source_name, error, discovered)?;
        return Ok((ok_schema, Some((options.error_status, error_schema))));
    }
    let response = project_type(context, &value.source_name, output, discovered)?;
    Ok((response, None))
}

/// If `tpe` is `morphir/SDK:result#result` applied to exactly two
/// arguments, its `(error, value)` type arguments — matching the argument
/// order `morphir-avro-extension`'s `project_result` uses, `Result error
/// value`, not `Result value error`.
fn result_arguments(tpe: &TypeExpr) -> Option<(&TypeExpr, &TypeExpr)> {
    match tpe {
        TypeExpr::Reference {
            source_name,
            arguments,
        } if source_name == SDK_RESULT && arguments.len() == 2 => {
            Some((&arguments[0], &arguments[1]))
        }
        _ => None,
    }
}

/// Apply one `options.operations` override to an already-projected
/// [`Operation`]: `method` and `path` replace their default, and each bound
/// parameter moves from `request` to `parameters`.
///
/// A `Path`-bound parameter must appear as a `{name}` placeholder in the
/// operation's path (the override's path when given, the default path
/// otherwise) once `path` itself has already been applied above; failing
/// that is `SchemaDiagnostic::unknown_operation` (`OAS002`), the same code
/// an override naming no value specification uses, because both are the
/// same failure at heart: `options.operations` describes an operation that
/// cannot exist as written. A parameter name that matches no request field
/// — for instance because `value`'s own inputs changed since the override
/// was written — is silently ignored rather than erroring: `options.operations`
/// is keyed by value FQName, not by parameter name, so there is no companion
/// diagnostic code reserved for it, and ignoring it leaves the field in the
/// request body, a safe default.
fn apply_override(
    operation: &mut Operation,
    override_: &OperationOverride,
) -> Result<(), SchemaDiagnostic> {
    if let Some(method) = override_.method {
        operation.method = method;
    }
    if let Some(path) = &override_.path {
        operation.path = path.clone();
    }

    for (name, binding) in &override_.parameters {
        if matches!(binding, ParameterBinding::Body) {
            continue;
        }
        if matches!(binding, ParameterBinding::Path) {
            let placeholder = format!("{{{name}}}");
            if !operation.path.contains(&placeholder) {
                return Err(SchemaDiagnostic::unknown_operation(
                    &operation.source_name,
                    format!(
                        "parameter '{name}' is bound to Path but '{}' has no '{placeholder}' placeholder",
                        operation.path
                    ),
                ));
            }
        }
        if let Some(index) = operation
            .request
            .iter()
            .position(|field| &field.name == name)
        {
            let field = operation.request.remove(index);
            operation.parameters.push((*binding, field));
        }
    }
    Ok(())
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

/// Drop any operation whose request or response refers to a name that is
/// not in `definitions`, and warn about it by its own Morphir FQName.
///
/// `extend_definitions`'s `close_definitions` call sweeps `definitions`
/// itself, so a type reachable only through some other type's closure never
/// survives with a dangling internal reference. An operation is a second,
/// independent source of references into that same `components/schemas`
/// namespace, sitting outside `definitions` — so once that sweep settles,
/// this is the equivalent sweep for `operations`: the same
/// `Unsupported::WarnAndSkip` rule (omit and warn by FQName) applied one
/// layer up, so a rendered document can never carry a `$ref` with no
/// `components/schemas` entry behind it, whether the missing entry was
/// never registered or was removed by the dangling sweep.
///
/// Under `Unsupported::Error` this never removes anything: any reference
/// this function would need to drop was already a `close_definitions`
/// failure that stopped the whole generation before reaching here.
fn drop_dangling_operations(
    operations: Vec<Operation>,
    definitions: &BTreeMap<String, NamedSchema>,
    diagnostics: &mut Vec<(SchemaDiagnostic, bool)>,
) -> Vec<Operation> {
    operations
        .into_iter()
        .filter(|operation| {
            let Some(missing) = operation_references(operation)
                .into_iter()
                .find(|name| !definitions.contains_key(*name))
            else {
                return true;
            };
            diagnostics.push((
                SchemaDiagnostic::unsupported_form(
                    &operation.source_name,
                    format!(
                        "its request or response refers to '{missing}', which was skipped, so it was skipped too"
                    ),
                ),
                true,
            ));
            false
        })
        .collect()
}

/// Every schema name an operation's request fields and response refer to.
fn operation_references(operation: &Operation) -> Vec<&str> {
    operation
        .request
        .iter()
        .flat_map(|field| references(&field.schema))
        .chain(
            operation
                .parameters
                .iter()
                .flat_map(|(_, field)| references(&field.schema)),
        )
        .chain(references(&operation.response))
        .chain(
            operation
                .error_response
                .iter()
                .flat_map(|(_, schema)| references(schema)),
        )
        .collect()
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

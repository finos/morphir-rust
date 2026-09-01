//! Render a [`SchemaProjection`] as an OpenAPI document.
//!
//! One document is produced per package, holding every named schema the
//! projection reached. `components/schemas` shares its bodies with the JSON
//! Schema renderer via [`crate::render::named_schema_body`]; only the `$ref`
//! base differs. The document is always built as OpenAPI 3.1 first and,
//! under [`crate::OpenApiVersion::V30`], rewritten to OpenAPI 3.0 afterward
//! — see [`render_openapi`].

use morphir_extension_sdk::Artifact;
use serde_json::{Map, Value, json};

use crate::render::downgrade::downgrade;
use crate::render::{named_schema_body, schema_body};
use crate::{
    HttpMethod, OpenApiVersion, Operation, ParameterBinding, Schema, SchemaDiagnostic, SchemaField,
    SchemaOptions, SchemaProjection, operation_id,
};
use morphir_projection::EntryPointKind;

/// The OpenAPI version string this renderer emits before any downgrade.
///
/// Every document is built in this dialect first, whatever
/// `SchemaOptions::version` asks for: [`OpenApiVersion::V30`] rewrites this
/// document afterward rather than being built separately, so there is one
/// projection and one document builder and the two versions cannot drift.
const OPENAPI_VERSION: &str = "3.1.0";

/// Where an OpenAPI document's own `$ref`s point.
const REF_PREFIX: &str = "#/components/schemas/";

/// Render `projection` as one OpenAPI document per package.
///
/// `options.projection` selects the public-model surface: in `schemas` mode
/// (`Projection::Schemas`), only `components/schemas` is populated and
/// `paths` is emitted as an empty object rather than omitted, because some
/// validators require the key. `options.version` selects the OpenAPI
/// dialect: [`OpenApiVersion::V31`] (the default) renders the document
/// built here unchanged; [`OpenApiVersion::V30`] rewrites it through
/// [`downgrade`], which can append `JSC003` warnings — a nullable
/// reference the 3.0 dialect cannot express — to `projection.diagnostics`.
/// `projection` is taken by mutable reference for exactly that: this
/// function otherwise only reads it.
pub fn render_openapi(
    projection: &mut SchemaProjection,
    options: &SchemaOptions,
) -> Result<Vec<Artifact>, SchemaDiagnostic> {
    Ok(vec![render_document(projection, options)?])
}

/// Render one OpenAPI document covering every schema the projection reached.
fn render_document(
    projection: &mut SchemaProjection,
    options: &SchemaOptions,
) -> Result<Artifact, SchemaDiagnostic> {
    let package_name = &projection.package_name;

    let mut info = Map::new();
    info.insert("title".to_owned(), json!(package_name));
    info.insert("version".to_owned(), json!("0.0.0"));
    info.insert("x-morphir-package".to_owned(), json!(package_name));

    let mut schemas = Map::new();
    for (name, named) in &projection.definitions {
        schemas.insert(
            name.clone(),
            Value::Object(named_schema_body(named, REF_PREFIX)),
        );
    }

    // In `schemas` mode `projection.operations` is empty, so `paths` still
    // renders as an empty object rather than being omitted, because some
    // validators require the key.
    let mut paths = Map::new();
    for operation in &projection.operations {
        let path_item = paths
            .entry(operation.path.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        let Value::Object(path_item) = path_item else {
            unreachable!("a path item is always inserted as an object");
        };
        path_item.insert(
            http_method_key(operation.method).to_owned(),
            render_operation(operation, REF_PREFIX),
        );
    }

    let mut components = Map::new();
    components.insert("schemas".to_owned(), Value::Object(schemas));

    let mut document = Map::new();
    document.insert("openapi".to_owned(), json!(OPENAPI_VERSION));
    document.insert("info".to_owned(), Value::Object(info));
    document.insert("paths".to_owned(), Value::Object(paths));
    document.insert("components".to_owned(), Value::Object(components));

    // Always build the 3.1 document first, whatever `options.version` asks
    // for, and rewrite it afterward: one projection, one document builder,
    // so the two versions cannot drift.
    let document = match options.version {
        OpenApiVersion::V31 => Value::Object(document),
        OpenApiVersion::V30 => {
            let (document, warnings) = downgrade(Value::Object(document))?;
            projection
                .diagnostics
                .extend(warnings.into_iter().map(|diagnostic| (diagnostic, true)));
            document
        }
    };

    Ok(Artifact {
        path: "openapi.json".to_owned(),
        content: format!(
            "{}\n",
            serde_json::to_string_pretty(&document)
                .expect("a document made of Value::Object and String always serializes")
        ),
        binary: false,
    })
}

/// Render one [`Operation`] as an OpenAPI Operation Object.
///
/// `x-morphir-value-kind` is read off whether `request` and `parameters` are
/// both empty rather than stored on `Operation` separately: a
/// [`morphir_projection::ValueKind::Constant`] is exactly the case with no
/// inputs at all, and an override that moves every input into `parameters`
/// still leaves it a function, so `request` alone is not enough once
/// overrides can empty it without the value itself being a constant.
fn render_operation(operation: &Operation, reference_base: &str) -> Value {
    let mut object = Map::new();
    object.insert(
        "operationId".to_owned(),
        json!(operation_id(&operation.source_name)),
    );
    object.insert("x-morphir-fqname".to_owned(), json!(operation.source_name));
    let value_kind = if operation.request.is_empty() && operation.parameters.is_empty() {
        "constant"
    } else {
        "function"
    };
    object.insert("x-morphir-value-kind".to_owned(), json!(value_kind));
    if let Some(doc) = &operation.doc {
        object.insert("description".to_owned(), json!(doc));
    }

    if !operation.parameters.is_empty() {
        let parameters = operation
            .parameters
            .iter()
            .map(|(binding, field)| render_parameter(*binding, field, reference_base))
            .collect();
        object.insert("parameters".to_owned(), Value::Array(parameters));
    }

    if !operation.request.is_empty() {
        let body_schema = Schema::Object {
            fields: operation.request.clone(),
            required: operation
                .request
                .iter()
                .map(|field| field.name.clone())
                .collect(),
        };
        let mut media = Map::new();
        media.insert(
            "schema".to_owned(),
            schema_body(&body_schema, reference_base),
        );
        let mut content = Map::new();
        content.insert("application/json".to_owned(), Value::Object(media));
        let mut request_body = Map::new();
        request_body.insert("required".to_owned(), json!(true));
        request_body.insert("content".to_owned(), Value::Object(content));
        object.insert("requestBody".to_owned(), Value::Object(request_body));
    }

    let mut response_media = Map::new();
    response_media.insert(
        "schema".to_owned(),
        schema_body(&operation.response, reference_base),
    );
    let mut response_content = Map::new();
    response_content.insert("application/json".to_owned(), Value::Object(response_media));
    let mut response_200 = Map::new();
    response_200.insert("description".to_owned(), json!("Successful result"));
    response_200.insert("content".to_owned(), Value::Object(response_content));
    let mut responses = Map::new();
    responses.insert("200".to_owned(), Value::Object(response_200));
    if let Some((status, schema)) = &operation.error_response {
        let mut error_media = Map::new();
        error_media.insert("schema".to_owned(), schema_body(schema, reference_base));
        let mut error_content = Map::new();
        error_content.insert("application/json".to_owned(), Value::Object(error_media));
        let mut error_response = Map::new();
        error_response.insert("description".to_owned(), json!("Error result"));
        error_response.insert("content".to_owned(), Value::Object(error_content));
        responses.insert(status.to_string(), Value::Object(error_response));
    }
    object.insert("responses".to_owned(), Value::Object(responses));

    if let Some(entry_point) = &operation.entry_point {
        object.insert("x-morphir-entry-point".to_owned(), json!(true));
        object.insert(
            "x-morphir-entry-point-id".to_owned(),
            json!(entry_point.identifier),
        );
        object.insert(
            "x-morphir-entry-point-kind".to_owned(),
            json!(entry_point_kind_key(entry_point.kind)),
        );
    }

    Value::Object(object)
}

/// The lowercase OpenAPI path-item key for an [`HttpMethod`].
fn http_method_key(method: HttpMethod) -> &'static str {
    match method {
        HttpMethod::Get => "get",
        HttpMethod::Put => "put",
        HttpMethod::Post => "post",
        HttpMethod::Delete => "delete",
        HttpMethod::Patch => "patch",
    }
}

/// The lowercase `x-morphir-entry-point-kind` value for an [`EntryPointKind`].
fn entry_point_kind_key(kind: EntryPointKind) -> &'static str {
    match kind {
        EntryPointKind::Main => "main",
        EntryPointKind::Command => "command",
        EntryPointKind::Handler => "handler",
    }
}

/// Render one [`Operation::parameters`] entry as an OpenAPI Parameter
/// Object.
///
/// Every Morphir input is required, so `required` is always `true`: an
/// override binds a field that already had to be present in the request
/// body to a parameter location instead, and moving where a value is
/// carried never makes it optional.
fn render_parameter(binding: ParameterBinding, field: &SchemaField, reference_base: &str) -> Value {
    let mut object = Map::new();
    object.insert("name".to_owned(), json!(field.name));
    object.insert("in".to_owned(), json!(parameter_location(binding)));
    object.insert("required".to_owned(), json!(true));
    object.insert(
        "schema".to_owned(),
        schema_body(&field.schema, reference_base),
    );
    if let Some(doc) = &field.doc {
        object.insert("description".to_owned(), json!(doc));
    }
    Value::Object(object)
}

/// The OpenAPI `in` value for a [`ParameterBinding`].
///
/// [`ParameterBinding::Body`] never reaches this: a `Body`-bound override
/// parameter is left in the request-body fields, so it never becomes a
/// [`Operation::parameters`] entry in the first place.
fn parameter_location(binding: ParameterBinding) -> &'static str {
    match binding {
        ParameterBinding::Path => "path",
        ParameterBinding::Query => "query",
        ParameterBinding::Header => "header",
        ParameterBinding::Body => unreachable!(
            "a Body-bound parameter stays in the request body and never becomes an Operation::parameters entry"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reproduces the bug this module used to have: with no roots to read a
    /// package name off of, `info.title` and `x-morphir-package` must still
    /// come out as the real package name, not empty, because they are read
    /// from `SchemaProjection::package_name` rather than reconstructed from
    /// a root's FQName.
    #[test]
    fn names_the_document_from_the_projection_even_with_no_roots() {
        let mut projection = SchemaProjection {
            package_name: "acme/customer".to_owned(),
            ..SchemaProjection::default()
        };
        let options = SchemaOptions::default();

        let artifacts = render_openapi(&mut projection, &options).expect("no unsupported forms");

        assert_eq!(artifacts.len(), 1);
        let document: Value = serde_json::from_str(&artifacts[0].content).expect("valid JSON");
        assert_eq!(document["info"]["title"], "acme/customer");
        assert_eq!(document["info"]["x-morphir-package"], "acme/customer");
        assert_eq!(document["components"]["schemas"], json!({}));
        assert_eq!(document["paths"], json!({}));
    }
}

//! Render a [`SchemaProjection`] as an OpenAPI 3.1 document.
//!
//! One document is produced per package, holding every named schema the
//! projection reached. `components/schemas` shares its bodies with the JSON
//! Schema renderer via [`crate::render::named_schema_body`]; only the `$ref`
//! base differs.

use morphir_extension_sdk::Artifact;
use serde_json::{Map, Value, json};

use crate::render::{named_schema_body, schema_body};
use crate::{HttpMethod, Operation, Schema, SchemaOptions, SchemaProjection, operation_id};
use morphir_projection::EntryPointKind;

/// The OpenAPI version string this renderer emits in `schemas` mode.
///
/// `SchemaOptions::version` also carries `OpenApiVersion::V30`, but its
/// downgrade behavior belongs to a later plan step; this renderer always
/// produces the 3.1 document that step downgrades from.
const OPENAPI_VERSION: &str = "3.1.0";

/// Where an OpenAPI document's own `$ref`s point.
const REF_PREFIX: &str = "#/components/schemas/";

/// Render `projection` as one OpenAPI document per package.
///
/// `options.projection` selects the public-model surface: in `schemas` mode
/// (`Projection::Schemas`), only `components/schemas` is populated and
/// `paths` is emitted as an empty object rather than omitted, because some
/// validators require the key.
pub fn render_openapi(projection: &SchemaProjection, options: &SchemaOptions) -> Vec<Artifact> {
    vec![render_document(projection, options)]
}

/// Render one OpenAPI document covering every schema the projection reached.
///
/// `options` is unused today: every option this task can observe
/// (`Projection::Schemas`, `OpenApiVersion::V31`) renders the same way, and
/// the rest — operation projection, the 3.0 downgrade — belongs to later
/// plan steps. It stays a parameter because those steps read it here.
fn render_document(projection: &SchemaProjection, _options: &SchemaOptions) -> Artifact {
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

    Artifact {
        path: "openapi.json".to_owned(),
        content: format!(
            "{}\n",
            serde_json::to_string_pretty(&Value::Object(document))
                .expect("a document made of Value::Object and String always serializes")
        ),
        binary: false,
    }
}

/// Render one [`Operation`] as an OpenAPI Operation Object.
///
/// `x-morphir-value-kind` is read off whether `request` is empty rather than
/// stored on `Operation` separately: a [`morphir_projection::ValueKind::Constant`]
/// is exactly the case with no inputs and so no request body, which is the
/// same condition either way.
fn render_operation(operation: &Operation, reference_base: &str) -> Value {
    let mut object = Map::new();
    object.insert(
        "operationId".to_owned(),
        json!(operation_id(&operation.source_name)),
    );
    object.insert("x-morphir-fqname".to_owned(), json!(operation.source_name));
    let value_kind = if operation.request.is_empty() {
        "constant"
    } else {
        "function"
    };
    object.insert("x-morphir-value-kind".to_owned(), json!(value_kind));
    if let Some(doc) = &operation.doc {
        object.insert("description".to_owned(), json!(doc));
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
        let projection = SchemaProjection {
            package_name: "acme/customer".to_owned(),
            ..SchemaProjection::default()
        };
        let options = SchemaOptions::default();

        let artifacts = render_openapi(&projection, &options);

        assert_eq!(artifacts.len(), 1);
        let document: Value = serde_json::from_str(&artifacts[0].content).expect("valid JSON");
        assert_eq!(document["info"]["title"], "acme/customer");
        assert_eq!(document["info"]["x-morphir-package"], "acme/customer");
        assert_eq!(document["components"]["schemas"], json!({}));
        assert_eq!(document["paths"], json!({}));
    }
}

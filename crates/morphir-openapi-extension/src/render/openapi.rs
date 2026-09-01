//! Render a [`SchemaProjection`] as an OpenAPI 3.1 document.
//!
//! One document is produced per package, holding every named schema the
//! projection reached. `components/schemas` shares its bodies with the JSON
//! Schema renderer via [`crate::render::named_schema_body`]; only the `$ref`
//! base differs.

use morphir_extension_sdk::Artifact;
use serde_json::{Map, Value, json};

use crate::render::named_schema_body;
use crate::{SchemaOptions, SchemaProjection};

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

    // Task 3 and Task 4 populate `paths` from `options.projection`. In
    // `schemas` mode there are no operations to project, and the key is
    // still emitted — empty rather than omitted — because some validators
    // require it.
    let paths = Map::new();

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

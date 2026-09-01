//! Renderers that turn a dialect-neutral [`crate::SchemaProjection`] into a
//! dialect-specific document.
//!
//! Today only [`json_schema`] renders. A follow-up plan adds an OpenAPI
//! renderer over the same projection, sharing the schema-body conversion
//! with a different `$ref` base.

pub mod json_schema;

pub use json_schema::render_json_schema;

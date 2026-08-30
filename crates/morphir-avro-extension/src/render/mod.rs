//! Deterministic serialization of the checked Avro projection model.

mod idl;
mod json;

pub use idl::render_idl;
pub use json::render_json;

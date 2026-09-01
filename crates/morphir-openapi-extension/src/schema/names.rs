//! Stable schema names derived from canonical Morphir names.
//!
//! Every name here is a pure function of the canonical Morphir name it is
//! derived from, so a projection does not depend on traversal order, and the
//! JSON Schema and OpenAPI renderers agree on the name a `$ref` must target.

use morphir_projection::{lower_camel, upper_camel};

/// Map a canonical Morphir FQName to a stable schema name.
///
/// `acme/customer:customer#customer-id` becomes `CustomerId`.
pub fn schema_name(source_name: &str) -> String {
    upper_camel(local_name(source_name))
}

/// Map a canonical Morphir name to a field or property name.
pub fn field_name(name: &str) -> String {
    lower_camel(name)
}

/// Map a canonical Morphir constructor name to a variant name.
pub fn variant_name(name: &str) -> String {
    upper_camel(local_name(name))
}

/// Take the local part of a canonical Morphir FQName.
fn local_name(source_name: &str) -> &str {
    source_name.rsplit('#').next().unwrap_or(source_name)
}

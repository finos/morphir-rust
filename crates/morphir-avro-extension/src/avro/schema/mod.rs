mod declarations;
mod names;
mod package;
mod protocol;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use serde_json::Value;

/// Deterministically ordered custom properties attached to an Avro model node.
pub type Properties = BTreeMap<String, Value>;

pub use declarations::{AvroField, EnumSchema, FixedSchema, NamedSchema, RecordSchema};
pub use names::{AvroFullName, AvroType, AvroUnion, UnionError};
pub use package::AvroPackage;
pub use protocol::{AvroMessage, AvroRequest, AvroRoot, Protocol};

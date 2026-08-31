use std::collections::BTreeMap;

use crate::AvroDiagnostic;

// `upper_camel`/`lower_camel` live in `morphir-projection` so that
// normalization (synthetic argument names) and this Avro renderer derive
// identifiers with the exact same transform. Re-exported here, rather than
// redefined, so every existing `crate::naming::{upper_camel, lower_camel}`
// call site in this crate keeps working unchanged.
pub(crate) use morphir_projection::{lower_camel, upper_camel};

const AVRO_KEYWORDS: &[&str] = &[
    "array",
    "big_decimal",
    "boolean",
    "bytes",
    "date",
    "decimal",
    "double",
    "enum",
    "error",
    "false",
    "fixed",
    "float",
    "idl",
    "import",
    "int",
    "local_timestamp_ms",
    "long",
    "map",
    "namespace",
    "null",
    "oneway",
    "protocol",
    "record",
    "schema",
    "string",
    "throws",
    "time_ms",
    "timestamp_ms",
    "true",
    "union",
    "uuid",
    "void",
];

/// Render a semantic Avro identifier for Avro IDL.
///
/// Avro JSON uses the semantic identifier unchanged. IDL keywords are wrapped
/// in backticks only when the IDL representation is rendered.
pub fn escape_idl_identifier(identifier: &str) -> String {
    if AVRO_KEYWORDS.contains(&identifier) {
        format!("`{identifier}`")
    } else {
        identifier.to_owned()
    }
}

pub(crate) fn namespace(package: &str, module: &[String]) -> String {
    package
        .split('/')
        .chain(module.iter().map(String::as_str))
        .filter(|part| !part.is_empty())
        .map(lower_camel)
        .collect::<Vec<_>>()
        .join(".")
}

pub(crate) fn full_name_from_source(source: &str) -> Option<(String, String)> {
    let (qualified, local) = source.rsplit_once('#')?;
    let (package, module) = qualified.split_once(':')?;
    let module = module.split('/').map(str::to_owned).collect::<Vec<_>>();
    Some((namespace(package, &module), upper_camel(local)))
}

pub(crate) fn is_valid_identifier(identifier: &str) -> bool {
    let mut bytes = identifier.bytes();
    matches!(bytes.next(), Some(first) if first == b'_' || first.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct NameRegistry {
    claimed: BTreeMap<String, Vec<u8>>,
}

impl NameRegistry {
    pub(crate) fn claim(
        &mut self,
        full_name: &str,
        source_identity: &str,
    ) -> Result<(), AvroDiagnostic> {
        self.claim_bytes(full_name, source_identity.as_bytes())
    }

    pub(crate) fn claim_bytes(
        &mut self,
        full_name: &str,
        source_identity: &[u8],
    ) -> Result<(), AvroDiagnostic> {
        match self.claimed.get(full_name) {
            None => {
                self.claimed
                    .insert(full_name.to_owned(), source_identity.to_owned());
                Ok(())
            }
            Some(existing) if existing == source_identity => Ok(()),
            Some(_) => Err(AvroDiagnostic::name_collision(full_name)),
        }
    }
}

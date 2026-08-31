use std::collections::BTreeMap;

use crate::AvroDiagnostic;

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

pub(crate) fn upper_camel(source: &str) -> String {
    let words = words(source);
    let result = words
        .iter()
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<String>();
    valid_identifier(result)
}

pub(crate) fn lower_camel(source: &str) -> String {
    let upper = upper_camel(source);
    let mut chars = upper.chars();
    chars
        .next()
        .map(|first| first.to_lowercase().chain(chars).collect::<String>())
        .unwrap_or_else(|| "_".to_owned())
}

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

fn words(source: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous_was_lowercase_or_digit = false;
    for character in source.chars() {
        if !character.is_ascii_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            previous_was_lowercase_or_digit = false;
            continue;
        }
        if character.is_ascii_uppercase() && previous_was_lowercase_or_digit && !current.is_empty()
        {
            words.push(std::mem::take(&mut current));
        }
        previous_was_lowercase_or_digit =
            character.is_ascii_lowercase() || character.is_ascii_digit();
        current.push(character.to_ascii_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn valid_identifier(mut value: String) -> String {
    if value.is_empty() {
        value.push('_');
    }
    if value.as_bytes()[0].is_ascii_digit() {
        value.insert(0, '_');
    }
    value
}

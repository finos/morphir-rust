//! Shared camelCase identifier helpers.
//!
//! These derive synthetic and semantic identifiers (record fields, synthetic
//! argument names, namespace segments) from Morphir source names. Every
//! backend extension that projects [`crate::ProjectionPackage`] into a
//! target schema language is expected to call these directly (or, for Avro,
//! re-export them) rather than reimplement the transform: normalization and
//! rendering must agree byte-for-byte on the derived name for a given
//! backend, so this is the single source of truth both sides depend on
//! producing identical output for.

/// Convert `source` to `UpperCamelCase`, treating any run of non-alphanumeric
/// characters as a word boundary.
pub fn upper_camel(source: &str) -> String {
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

/// Convert `source` to `lowerCamelCase`, treating any run of non-alphanumeric
/// characters as a word boundary.
pub fn lower_camel(source: &str) -> String {
    let upper = upper_camel(source);
    let mut chars = upper.chars();
    chars
        .next()
        .map(|first| first.to_lowercase().chain(chars).collect::<String>())
        .unwrap_or_else(|| "_".to_owned())
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

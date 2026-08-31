//! Pure camelCase identifier helpers used by normalization to derive
//! synthetic argument names for curried value signatures.
//!
//! This is a private, verbatim copy of the same helpers in
//! `morphir-avro-extension::naming`. Only the case-conversion logic is
//! duplicated here; nothing Avro-specific (IDL escaping, namespaces, or
//! `AvroDiagnostic`) is included, so this module has no dependency on the
//! Avro crate.

pub(super) fn upper_camel(source: &str) -> String {
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

pub(super) fn lower_camel(source: &str) -> String {
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

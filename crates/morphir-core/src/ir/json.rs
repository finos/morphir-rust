//! The JSON profile's canonical text form.

use serde_json::Value as Json;

/// Writes a value in the JSON profile's canonical text form.
///
/// The profile's writer is not `serde_json::to_string`: a non-empty object is padded inside its
/// braces and its members separated by `, `, while an array is not padded. The driver compares
/// canonicals as strings (kit README, "What the driver does with a case"), so this is part of
/// the contract rather than a style.
pub fn write_canonical(value: &Json) -> String {
    match value {
        Json::Null => "null".to_string(),
        Json::Bool(true) => "true".to_string(),
        Json::Bool(false) => "false".to_string(),
        // What `arbitrary_precision` buys is that a number *parsed from text* keeps the lexeme
        // it was written with, which is what a `DocumentLiteral` payload needs. `Literal::Float`
        // now carries its lexeme too, so `1.0e2` comes back out as `1.0e2` rather than `100.0`.
        Json::Number(number) => number.to_string(),
        Json::String(_) => serde_json::to_string(value).expect("a string always serializes"),
        Json::Array(elements) if elements.is_empty() => "[]".to_string(),
        Json::Array(elements) => format!(
            "[{}]",
            elements
                .iter()
                .map(write_canonical)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Json::Object(members) if members.is_empty() => "{}".to_string(),
        Json::Object(members) => format!(
            "{{ {} }}",
            members
                .iter()
                .map(|(key, member)| format!(
                    "{}: {}",
                    serde_json::to_string(key).expect("a member name always serializes"),
                    write_canonical(member)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

//! Deterministic serialization of the checked Avro projection model.

mod idl;
mod json;

pub use idl::render_idl;
pub use json::render_json;

fn portable_artifact_component(component: &str) -> String {
    let uppercase = component.to_ascii_uppercase();
    let reserved = matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            uppercase.strip_prefix(prefix).is_some_and(|number| {
                number.len() == 1 && matches!(number.as_bytes()[0], b'1'..=b'9')
            })
        });
    if reserved {
        format!("~{component}")
    } else {
        component.to_owned()
    }
}

//! Deterministic serialization of the checked Avro projection model.

mod idl;
mod json;

use sha2::{Digest, Sha256};

use crate::AvroFullName;

pub use idl::render_idl;
pub use json::render_json;

const MAX_PORTABLE_COMPONENT_UNITS: usize = 255;
const MAX_PORTABLE_PATH_UNITS: usize = 1_024;
const DIGEST_BYTES: usize = 16;

fn portable_artifact_path(name: &AvroFullName, extension: &str) -> String {
    let namespace = name
        .namespace()
        .split('.')
        .filter(|component| !component.is_empty())
        .map(|component| portable_artifact_component(component, MAX_PORTABLE_COMPONENT_UNITS))
        .collect::<Vec<_>>();
    let leaf_budget = MAX_PORTABLE_COMPONENT_UNITS - extension.len() - 1;
    let leaf = portable_artifact_component(name.name(), leaf_budget);
    let mut components = namespace.clone();
    components.push(leaf.clone());
    let path = format!("{}.{}", components.join("/"), extension);
    if path.len() <= MAX_PORTABLE_PATH_UNITS {
        return path;
    }

    let namespace = format!("~ns~{}", digest_suffix(name.namespace()));
    format!("{namespace}/{leaf}.{extension}")
}

fn portable_artifact_component(component: &str, max_units: usize) -> String {
    let uppercase = component.to_ascii_uppercase();
    let reserved = matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            uppercase.strip_prefix(prefix).is_some_and(|number| {
                number.len() == 1 && matches!(number.as_bytes()[0], b'1'..=b'9')
            })
        });
    let escaped = if reserved {
        format!("~{component}")
    } else {
        component.to_owned()
    };
    if escaped.len() <= max_units {
        return escaped;
    }

    let suffix = format!("~{}", digest_suffix(component));
    let prefix_len = max_units - suffix.len();
    format!("{}{}", &escaped[..prefix_len], suffix)
}

fn digest_suffix(value: &str) -> String {
    Sha256::digest(value.as_bytes())[..DIGEST_BYTES]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

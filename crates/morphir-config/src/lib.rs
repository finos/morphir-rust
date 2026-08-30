//! Portable, filesystem-free Morphir configuration algorithms.

pub mod env;
pub mod legacy;
pub mod merge;
mod parse;
pub mod secret;

use serde_json::{Value, json};

pub use env::env_config_value;
pub use merge::{ProvenanceMap, ValuePath, deep_merge, deep_merge_with_provenance, merge_all};
pub use parse::{ConfigParseError, parse_config};
pub use secrecy::{ExposeSecret, SecretString};
pub use secret::{SecretReference, SecretReferenceError, is_secret_reference};

/// Return the lowest-precedence configuration layer built into Morphir tools.
///
/// Sections whose presence carries meaning, such as `project` and `workspace`,
/// are intentionally omitted.
pub fn builtin_defaults() -> Value {
    json!({
        "frontend": {
            "emit_parse_stage": true,
            "emit_parse_stage_fatal": false,
        },
        "ir": {
            "format_version": 4,
            "mode": "vfs",
            "strict_mode": false,
        },
        "codegen": {
            "targets": [],
            "output_format": "pretty",
        },
    })
}

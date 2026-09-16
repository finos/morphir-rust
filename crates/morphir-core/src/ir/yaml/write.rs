//! The canonical YAML writer.
//!
//! See `docs/spec/ir/schemas/v4/yaml-profile.md`, "Canonical writer". This is a stub: the writer
//! is the next task in the stage-2 plan, and it lives here so the module's shape is settled.

/// Writes a JSON value tree as canonical profile YAML.
pub fn write_canonical(_value: &serde_json::Value) -> String {
    unimplemented!("the canonical YAML writer is the next task in the stage-2 plan")
}

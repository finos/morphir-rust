//! Convert draft.1 records at the boundary, before validating current member paths.

use super::ExtensionSchemaVersion;
use morphir_extension_sdk::claims::{CLAIMS_VERSION, CapabilityClaimSet};
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Draft {
    One,
    Two,
}

/// Validate the enclosing schema before interpreting artifact or catalog members.
pub(crate) fn normalize_envelope(value: &mut Value, children: &str) -> Result<(), String> {
    let version: ExtensionSchemaVersion = value
        .get("schemaVersion")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    let draft = match version {
        ExtensionSchemaVersion::Semver(version) if version.major == 2 => {
            Some(if version.pre.as_str() == "draft.1" {
                Draft::One
            } else {
                Draft::Two
            })
        }
        _ => None,
    };
    normalize_record_for(value, draft)?;
    if let Some(Value::Array(records)) = value.get_mut(children) {
        for record in records {
            normalize_record_for(record, draft)?;
        }
    }
    if draft.is_some() {
        value["schemaVersion"] = Value::String("2.0.0-draft.2".into());
    }
    Ok(())
}

/// Standalone records have no enclosing schema, so recognize either complete shape.
pub(crate) fn normalize_record(value: &mut Value) -> Result<(), String> {
    if value.get("schemaVersion").is_some() {
        return normalize_envelope(value, "");
    }
    normalize_record_for(value, None)
}

fn normalize_record_for(value: &mut Value, draft: Option<Draft>) -> Result<(), String> {
    let object = value.as_object_mut().ok_or("expected extension record")?;
    let old = object.contains_key("statement") || object.contains_key("statementSource");
    let new = object.contains_key("claims") || object.contains_key("claimCheck");
    if (old && new) || (old && draft == Some(Draft::Two)) || (new && draft == Some(Draft::One)) {
        return Err(
            "capability claim set members do not match schemaVersion (mixed draft record)".into(),
        );
    }
    if let Some(document) = object.get(if old { "statement" } else { "claims" }) {
        let old_document = document.get("statementVersion").is_some();
        if old != old_document {
            return Err("capability claim set version member does not match record members (mixed draft record)".into());
        }
    }
    if old {
        if let Some(document) = object.remove("statement") {
            object.insert("claims".into(), document);
        }
        if let Some(source) = object.remove("statementSource") {
            let check = match source.as_str() {
                Some("declared") => Value::String("unchecked".into()),
                Some("probed") => source,
                _ => return Err("invalid draft.1 claim check; expected declared or probed".into()),
            };
            object.insert("claimCheck".into(), check);
        }
    }
    // Only rewrite known record-level paths. Unknown optional payloads are opaque.
    if let Some(Value::Array(paths)) = object.get_mut("critical") {
        for path in paths {
            if let Some(text) = path.as_str() {
                let mut parts: Vec<_> = text.split('.').collect();
                let member = if matches!(parts[0], "artifacts" | "extensions") {
                    1
                } else {
                    0
                };
                if let Some(part) = parts.get_mut(member) {
                    let is_old = matches!(*part, "statement" | "statementSource");
                    let is_new = matches!(*part, "claims" | "claimCheck");
                    if (is_old && draft == Some(Draft::Two))
                        || (is_new && draft == Some(Draft::One))
                    {
                        return Err("critical claims path does not match schemaVersion (mixed draft record)".into());
                    }
                    *part = match *part {
                        "statement" => "claims",
                        "statementSource" => "claimCheck",
                        other => other,
                    };
                    if is_old && parts.get(member + 1) == Some(&"claimsVersion") {
                        return Err(
                            "unknown critical claimsVersion member in draft.1 record".into()
                        );
                    }
                    if is_old && parts.get(member + 1) == Some(&"statementVersion") {
                        parts[member + 1] = "claimsVersion";
                    }
                }
                *path = Value::String(parts.join("."));
            }
        }
    }
    if let Some(document) = object.get_mut("claims") {
        normalize_document::<serde_json::Error>(document).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Keep unknown optional fields while updating the validated document's wire spelling.
pub(super) fn normalize_document<E: serde::de::Error>(wire: &mut Value) -> Result<(), E> {
    let parsed: CapabilityClaimSet = serde_json::from_value(wire.clone()).map_err(E::custom)?;
    let object = wire
        .as_object_mut()
        .expect("validated claim set is an object");
    object.remove("statementVersion");
    object.insert("claimsVersion".into(), Value::String(CLAIMS_VERSION.into()));
    if object.contains_key("critical") {
        object.insert(
            "critical".into(),
            serde_json::to_value(parsed.critical).map_err(E::custom)?,
        );
    }
    Ok(())
}

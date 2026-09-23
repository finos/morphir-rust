//! Compatibility rules shared only by extension distribution documents.

mod statement;
mod version;

pub use statement::{ProbeSource, StatementProvenance, StatementRecord};
pub use version::ExtensionSchemaVersion;

use morphir_extension_sdk::statement::{CapabilityStatement, StatementRequirements};
use serde::Deserialize;
use serde_json::Value;

pub(crate) fn validate_members(
    value: &Value,
    known_paths: &[&str],
    validate_requires: bool,
) -> Result<(), String> {
    let critical: Vec<String> = value.get("critical").map_or_else(
        || Ok(Vec::new()),
        |value| serde_json::from_value(value.clone()).map_err(|error| error.to_string()),
    )?;
    for path in &critical {
        let statement_path = path
            .split_once("statement.")
            .filter(|(prefix, _)| known_paths.contains(&format!("{prefix}statement").as_str()));
        if !known_paths.contains(&path.as_str())
            && !statement_path
                .is_some_and(|(_, path)| morphir_extension_sdk::statement::understands_member(path))
        {
            return Err(format!("unknown critical member '{path}'"));
        }
    }
    if validate_requires && let Some(requires) = value.get("requires") {
        let _: StatementRequirements =
            serde_json::from_value(requires.clone()).map_err(|error| error.to_string())?;
        if requires.get("host").is_some() && !critical.iter().any(|path| path == "requires.host") {
            return Err("requires.host must be listed in critical".into());
        }
    }
    Ok(())
}

pub(crate) const CAPABILITY_PATHS: &[&str] = &[
    "frontend",
    "frontend.languages",
    "frontend.languages.id",
    "frontend.languages.fileExtensions",
    "frontend.irVersions",
    "frontend.compile",
    "frontend.incremental",
    "backend",
    "backend.targets",
    "backend.irVersions",
    "backend.generate",
];

/// Read a shared strict type with extension-specific must-ignore semantics.
/// Tool records still use the strict deserializer directly.
pub(crate) fn read_platform<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<crate::Platform>, D::Error> {
    #[derive(serde::Deserialize)]
    struct Platform {
        os: String,
        arch: String,
    }
    let value = Option::<Platform>::deserialize(deserializer)?;
    value
        .map(|value| crate::Platform::new(value.os, value.arch).map_err(serde::de::Error::custom))
        .transpose()
}

/// Check requirements already validated by a distribution document reader.
pub(crate) fn check_requirements(
    requires: Option<&Value>,
    host: &semver::Version,
) -> crate::Result<()> {
    let mut statement =
        CapabilityStatement::from_session(vec![], Default::default(), Default::default());
    statement.requires = requires.map(|value| {
        serde_json::from_value(value.clone()).expect("reader validated host requirements")
    });
    statement.check_host(host).map_err(Into::into)
}

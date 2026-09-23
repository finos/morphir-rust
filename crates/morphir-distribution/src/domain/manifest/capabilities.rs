//! Validated flat capability metadata used by version-1 records.

use super::*;

/// One source language accepted by a schema `"1.0"` frontend extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendLanguageRecord {
    id: String,
    file_extensions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrontendLanguageRecordWire {
    id: String,
    file_extensions: Vec<String>,
}

impl<'de> Deserialize<'de> for FrontendLanguageRecord {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = FrontendLanguageRecordWire::deserialize(deserializer)?;
        let id = wire.id.trim();
        if id.is_empty() || id != wire.id {
            return Err(serde::de::Error::custom(
                "frontend languages must have non-empty trimmed IDs",
            ));
        }
        if !valid_frontend_file_extensions(&wire.file_extensions) {
            return Err(serde::de::Error::custom(
                "frontend file extensions must be non-empty, dot-prefixed, trimmed, and unique",
            ));
        }
        Ok(Self {
            id: wire.id,
            file_extensions: wire.file_extensions,
        })
    }
}

impl FrontendLanguageRecord {
    /// Return the stable source-language identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Return the non-empty unique file extensions recognized for this language.
    pub fn file_extensions(&self) -> &[String] {
        &self.file_extensions
    }
}

/// Frontend-specific metadata carried by schema `"1.0"` release records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendRecord {
    languages: Vec<FrontendLanguageRecord>,
    ir_versions: Vec<String>,
    #[serde(default = "default_frontend_compile")]
    compile: bool,
    /// Whether the frontend accepts a baseline and reports per-module results.
    /// Absent in records written before incremental frontends existed, and in
    /// every record for a frontend that is not incremental.
    #[serde(default, skip_serializing_if = "is_false")]
    incremental: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrontendRecordWire {
    languages: Vec<FrontendLanguageRecord>,
    ir_versions: Vec<String>,
    #[serde(default = "default_frontend_compile")]
    compile: bool,
    #[serde(default)]
    incremental: bool,
}

fn default_frontend_compile() -> bool {
    true
}

fn valid_frontend_file_extensions(values: &[String]) -> bool {
    !values.is_empty()
        && values.iter().all(|value| {
            let trimmed = value.trim();
            !trimmed.is_empty() && trimmed == value && trimmed.starts_with('.')
        })
        && values
            .iter()
            .map(|value| value.trim())
            .collect::<BTreeSet<_>>()
            .len()
            == values.len()
}

impl<'de> Deserialize<'de> for FrontendRecord {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = FrontendRecordWire::deserialize(deserializer)?;
        if wire.languages.is_empty()
            || wire
                .languages
                .iter()
                .map(|language| language.id().trim())
                .collect::<BTreeSet<_>>()
                .len()
                != wire.languages.len()
        {
            return Err(serde::de::Error::custom(
                "frontend languages must be non-empty and have unique IDs",
            ));
        }
        if !valid_backend_identifiers(&wire.ir_versions) {
            return Err(serde::de::Error::custom(
                "frontend IR versions must be non-empty and unique",
            ));
        }
        Ok(Self {
            languages: wire.languages,
            ir_versions: wire.ir_versions,
            compile: wire.compile,
            incremental: wire.incremental,
        })
    }
}

impl FrontendRecord {
    /// Return the non-empty set of source languages accepted by the frontend.
    pub fn languages(&self) -> &[FrontendLanguageRecord] {
        &self.languages
    }

    /// Return the non-empty unique Morphir IR versions produced by the frontend.
    pub fn ir_versions(&self) -> &[String] {
        &self.ir_versions
    }

    /// Return whether this frontend accepts compile requests.
    pub fn compile(&self) -> bool {
        self.compile
    }

    /// Return whether this frontend compiles incrementally against a baseline.
    pub fn incremental(&self) -> bool {
        self.incremental
    }
}

/// Backend-specific metadata carried by schema `"1.0"` release records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendRecord {
    targets: Vec<String>,
    ir_versions: Vec<String>,
    generate: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackendRecordWire {
    targets: Vec<String>,
    ir_versions: Vec<String>,
    #[serde(default = "default_backend_generate")]
    generate: bool,
}

fn default_backend_generate() -> bool {
    true
}

fn valid_backend_identifiers(values: &[String]) -> bool {
    !values.is_empty()
        && values.iter().all(|value| {
            let trimmed = value.trim();
            !trimmed.is_empty() && trimmed == value
        })
        && values
            .iter()
            .map(|value| value.trim())
            .collect::<BTreeSet<_>>()
            .len()
            == values.len()
}

impl<'de> Deserialize<'de> for BackendRecord {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BackendRecordWire::deserialize(deserializer)?;
        if !valid_backend_identifiers(&wire.targets) {
            return Err(serde::de::Error::custom(
                "backend targets must be non-empty and unique",
            ));
        }
        if !valid_backend_identifiers(&wire.ir_versions) {
            return Err(serde::de::Error::custom(
                "backend IR versions must be non-empty and unique",
            ));
        }
        Ok(Self {
            targets: wire.targets,
            ir_versions: wire.ir_versions,
            generate: wire.generate,
        })
    }
}

impl BackendRecord {
    /// Return the non-empty unique backend target names.
    pub fn targets(&self) -> &[String] {
        &self.targets
    }

    /// Return the non-empty unique Morphir IR versions supported by the backend.
    pub fn ir_versions(&self) -> &[String] {
        &self.ir_versions
    }

    /// Return whether this backend accepts generate requests.
    pub fn generate(&self) -> bool {
        self.generate
    }
}

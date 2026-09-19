//! Offline draft 2020-12 schema compilation, separate from document rejection.

use crate::strict_json;
use jsonschema::{Draft, Registry, RegistryBuilder, Retrieve, Uri, Validator};
use serde_json::Value;

/// Which draft metadata artifact to validate.
#[derive(Debug, Clone, Copy)]
pub enum Artifact {
    /// Release manifest.
    Manifest,
    /// Lock-core graph projection.
    Lock,
}

/// Invalid schemas and unresolved references are infrastructure failures.
#[derive(Debug, thiserror::Error)]
#[error("package schema compilation failed: {0}")]
pub struct SchemaError(String);

struct Offline;
impl Retrieve for Offline {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err(format!("external schema retrieval is disabled: {uri}").into())
    }
}

/// Both driver-supplied schemas, compiled before examining any document.
pub struct PackageSchemas {
    manifest: Validator,
    lock: Validator,
}

impl PackageSchemas {
    /// Compile draft 2020-12 schemas with only the supplied resources available.
    pub fn compile(manifest: &Value, lock: &Value) -> Result<Self, SchemaError> {
        if !manifest.is_object() || !lock.is_object() {
            return Err(SchemaError("schemas must be objects".into()));
        }
        let resources = [manifest, lock].into_iter().filter_map(|schema| {
            schema
                .get("$id")
                .and_then(Value::as_str)
                .map(|id| (id, schema))
        });
        let registry = Registry::new()
            .draft(Draft::Draft202012)
            .retriever(Offline)
            .extend(resources)
            .and_then(RegistryBuilder::prepare)
            .map_err(|e| SchemaError(e.to_string()))?;
        let options = jsonschema::options()
            .with_draft(Draft::Draft202012)
            .with_retriever(Offline)
            .with_registry(&registry);
        let manifest = options
            .build(manifest)
            .map_err(|e| SchemaError(e.to_string()))?;
        let lock = options
            .build(lock)
            .map_err(|e| SchemaError(e.to_string()))?;
        Ok(Self { manifest, lock })
    }

    /// Validate JSON structure, refusing duplicate decoded keys.
    /// Normalization and graph consistency belong to Library-set verification.
    pub fn validate(&self, artifact: Artifact, input: &str) -> bool {
        strict_json::parse(input).is_ok_and(|value| self.is_valid(artifact, &value))
    }

    pub(crate) fn is_valid(&self, artifact: Artifact, value: &Value) -> bool {
        match artifact {
            Artifact::Manifest => &self.manifest,
            Artifact::Lock => &self.lock,
        }
        .is_valid(value)
    }
}

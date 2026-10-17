//! Wire bindings for the additive MorphirModel contract.
//!
//! These DTOs decode message shape. Session admission, integrity, limits and
//! evidence checking belong to the model validation boundary.

use crate::{CompileOptions, CompilePackage, Diagnostic, SourceSet};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;

/// An operation selected by a complete model transport tuple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelOperation {
    /// Compile source documents into a complete model.
    #[serde(rename = "morphir.frontend.compileModel")]
    CompileModel,
    /// Generate artifacts from a retained model.
    #[serde(rename = "morphir.backend.generateModel")]
    GenerateModel,
}

/// The physical codec of a complete document, independent of its guarantees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelCodec {
    /// Ion 1.0 binary with its version marker and local symbol tables.
    #[serde(rename = "ion-binary")]
    IonBinary,
    /// Morphir JSON encoded as UTF-8.
    #[serde(rename = "morphir-json")]
    MorphirJson,
}

/// The layout carried by a model payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelLayout {
    /// One complete document, without external references or compression.
    #[serde(rename = "single-document")]
    SingleDocument,
}

/// One complete supported combination, without an implied Cartesian product.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelTransportTuple {
    /// The exact model operation this combination authorizes.
    pub operation: ModelOperation,
    /// The model-content profile, independent of source and target language.
    pub profile: String,
    /// The selected physical codec.
    pub codec: ModelCodec,
    /// The independent exact model-envelope version.
    pub envelope_version: String,
    /// The selected semantic IR version.
    pub ir_version: String,
}

/// Raw wire limits; validation enforces positive values and the protocol bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelTransportLimits {
    /// Aggregate base64 content bytes in one request or response.
    pub max_encoded_bytes: u32,
    /// Aggregate decoded model, dependency and evidence bytes.
    pub max_decoded_bytes: u32,
    /// Maximum depth of each decoded document, with its root at depth one.
    pub max_depth: u32,
    /// Aggregate decoded value nodes across the message's payloads.
    pub max_nodes: u32,
}

/// A host offer or selected guest session support.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelTransportSupport {
    /// Complete combinations offered or selected for this session.
    pub tuples: Vec<ModelTransportTuple>,
    /// Resource bounds independently admitted by both peers.
    pub limits: ModelTransportLimits,
    /// Guarantee identifiers the peer can interpret, without establishing them.
    #[serde(default)]
    pub guarantees: Vec<String>,
}

/// Opaque bytes retained independently of codec and evidence admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPayload {
    /// The physical representation of the bytes.
    pub codec: ModelCodec,
    /// The layout of the decoded document.
    pub layout: ModelLayout,
    /// Exact byte length after base64 decoding.
    pub decoded_byte_length: u32,
    /// Lowercase SHA-256 over those exact decoded bytes.
    pub sha256: String,
    /// Canonical padded base64 content, including for the JSON codec.
    pub content: String,
}

/// A prerequisite for an exact model or node scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelGuaranteeRequirement {
    /// The namespaced guarantee identifier.
    pub guarantee: String,
    /// The exact model or node identity to which the guarantee must apply.
    pub scope: String,
}

/// Attributed evidence requiring an accepted checker or trust policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelGuaranteeEvidence {
    /// The namespaced guarantee claimed by this evidence.
    pub guarantee: String,
    /// The logical model identity to which this evidence applies.
    pub model_id: String,
    /// The immutable logical model revision being checked.
    pub model_revision: String,
    /// SHA-256 of the agreed dependency/configuration context.
    pub context_digest: String,
    /// The exact checked model or node scope.
    pub scope: String,
    /// Attribution identifying the producer, without authenticating it.
    pub producer: String,
    /// Independently encoded opaque evidence under the same aggregate bounds.
    pub evidence: ModelPayload,
}

/// A model envelope retaining semantic IR, enrichments and independent claims.
///
/// The profile owns payload meaning. Structural variants remain in the IR's
/// existing Distribution; this envelope does not introduce pipeline stage tags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MorphirModel {
    /// The independent exact envelope revision.
    pub envelope_version: String,
    /// A stable logical identity, distinct from payload bytes.
    pub model_id: String,
    /// An immutable logical revision, distinct from the transport digest.
    pub revision: String,
    /// The semantic IR version represented by the payload.
    pub ir_version: String,
    /// The content profile interpreting the opaque payload.
    pub profile: String,
    /// Complete bytes preserving unknown optional enrichments and annotations.
    pub payload: ModelPayload,
    /// Scoped evidence claims, not automatically established guarantees.
    #[serde(default)]
    pub evidence: Vec<ModelGuaranteeEvidence>,
    /// Exact registered envelope paths whose semantics a receiver must know.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub critical: Vec<String>,
}

/// A complete dependency model for source compilation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileModelDependency {
    /// The dependency's package namespace.
    pub package_name: String,
    /// Its full model, using the admitted dependency representation.
    pub model: MorphirModel,
}

/// Source-set compilation selecting an already admitted model output tuple.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileModelRequest {
    /// The advertised source language.
    pub language_id: String,
    /// Complete source documents and their optional naming root.
    pub sources: SourceSet,
    /// The package being compiled.
    pub package: CompilePackage,
    /// Complete dependency models, without a legacy JSON-only carrier.
    #[serde(default)]
    pub dependencies: Vec<CompileModelDependency>,
    /// Existing compilation options; IR version must match the output tuple.
    pub options: CompileOptions,
    /// Digest binding the declared dependency/configuration context.
    pub context_digest: String,
    /// The exact admitted compilation tuple.
    pub output: ModelTransportTuple,
}

/// A compilation outcome with success and failure represented separately.
///
/// Successful wire results require a model. Failed results forbid the model
/// member entirely, including a null value.
///
/// ```
/// use morphir_extension_sdk::model::CompileModelResult;
/// let failed = CompileModelResult::Failed { diagnostics: Vec::new() };
/// assert_eq!(serde_json::to_value(failed).unwrap(),
///            serde_json::json!({"success": false, "diagnostics": []}));
/// ```
#[derive(Debug, Clone)]
pub enum CompileModelResult {
    /// A successful compilation retaining the complete model.
    Compiled {
        /// The required output model.
        model: Box<MorphirModel>,
        /// Structured compilation observations.
        diagnostics: Vec<Diagnostic>,
        /// Compiled module names.
        modules: Vec<String>,
    },
    /// A compilation failure with no model.
    Failed {
        /// Structured diagnostics explaining the failure.
        diagnostics: Vec<Diagnostic>,
    },
}

impl Serialize for CompileModelResult {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Compiled {
                model,
                diagnostics,
                modules,
            } => {
                let mut map = serializer.serialize_map(Some(4))?;
                map.serialize_entry("success", &true)?;
                map.serialize_entry("model", model)?;
                map.serialize_entry("diagnostics", diagnostics)?;
                map.serialize_entry("modules", modules)?;
                map.end()
            }
            Self::Failed { diagnostics } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("success", &false)?;
                map.serialize_entry("diagnostics", diagnostics)?;
                map.end()
            }
        }
    }
}

/// Decode a present field without treating null as an absent optional member.
fn present_value<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

impl<'de> Deserialize<'de> for CompileModelResult {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct WireResult {
            success: bool,
            #[serde(default, deserialize_with = "present_value")]
            model: Option<MorphirModel>,
            #[serde(default)]
            diagnostics: Vec<Diagnostic>,
            #[serde(default, deserialize_with = "present_value")]
            modules: Option<serde_json::Value>,
        }

        let wire = WireResult::deserialize(deserializer)?;
        if wire.success {
            let model = wire
                .model
                .ok_or_else(|| serde::de::Error::missing_field("model"))?;
            let modules = wire
                .modules
                .map(serde_json::from_value)
                .transpose()
                .map_err(serde::de::Error::custom)?
                .unwrap_or_default();
            Ok(Self::Compiled {
                model: Box::new(model),
                diagnostics: wire.diagnostics,
                modules,
            })
        } else if wire.model.is_some() {
            Err(serde::de::Error::custom(
                "failed compilation forbids the model member",
            ))
        } else {
            Ok(Self::Failed {
                diagnostics: wire.diagnostics,
            })
        }
    }
}

/// Generation from a retained model and an explicitly selected consumer tuple.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateModelRequest {
    /// The complete model retained through the host pipeline.
    pub model: MorphirModel,
    /// The exact admitted generation tuple.
    pub input: ModelTransportTuple,
    /// The separately advertised output target.
    pub target: String,
    /// Digest binding dependency/configuration context for evidence admission.
    pub context_digest: String,
    /// Requested exact scoped prerequisites; consumer requirements still apply.
    #[serde(default)]
    pub required_guarantees: Vec<ModelGuaranteeRequirement>,
    /// Existing backend-specific options.
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

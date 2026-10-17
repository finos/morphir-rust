//! Bounded byte admission. Retention does not establish semantic guarantees.

use super::*;
use base64::Engine as _;
use sha2::{Digest, Sha256};

/// A stable admission reason without arbitrary payload contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelAdmissionError {
    code: &'static str,
    path: Option<String>,
}

impl ModelAdmissionError {
    pub(crate) const fn new(code: &'static str) -> Self {
        Self { code, path: None }
    }
    /// Machine-readable boundary failure; no model bytes or secrets are echoed.
    pub fn code(&self) -> &'static str {
        self.code
    }
    /// A bounded, escaped critical member path, when that caused the refusal.
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    fn unknown_critical(path: &str) -> Self {
        Self {
            code: "unknown_critical_path",
            path: Some(
                path.chars()
                    .flat_map(char::escape_default)
                    .take(160)
                    .collect(),
            ),
        }
    }
}

impl std::fmt::Display for ModelAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "model admission refused: {}", self.code)?;
        if let Some(path) = &self.path {
            write!(f, " at {path}")?;
        }
        Ok(())
    }
}
impl std::error::Error for ModelAdmissionError {}

/// A structurally admitted envelope retaining its exact original payload bytes.
///
/// This view proves transport integrity and profile structure only. Type
/// correctness and optimization prerequisites require the consumer's checker.
#[derive(Debug, Clone)]
pub struct RetainedModel {
    envelope: MorphirModel,
    bytes: Vec<u8>,
}

impl RetainedModel {
    /// The original envelope, including optional opaque evidence.
    pub fn envelope(&self) -> &MorphirModel {
        &self.envelope
    }
    /// The exact bytes whose digest was checked; no transcoding occurs.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

pub(super) struct Budget<'a> {
    pub(super) limits: &'a ModelTransportLimits,
    encoded: u64,
    decoded: u64,
    pub(super) nodes: u64,
}

impl<'a> Budget<'a> {
    pub(super) fn new(limits: &'a ModelTransportLimits) -> Result<Self, ModelAdmissionError> {
        if [
            limits.max_encoded_bytes,
            limits.max_decoded_bytes,
            limits.max_depth,
            limits.max_nodes,
        ]
        .iter()
        .any(|value| *value == 0 || *value > i32::MAX as u32)
        {
            return Err(ModelAdmissionError::new("invalid_limits"));
        }
        Ok(Self {
            limits,
            encoded: 0,
            decoded: 0,
            nodes: 0,
        })
    }

    fn payload(
        &mut self,
        payload: &ModelPayload,
        profile: Option<(&str, &str)>,
    ) -> Result<Vec<u8>, ModelAdmissionError> {
        self.encoded += payload.content.len() as u64;
        self.decoded += u64::from(payload.decoded_byte_length);
        if self.encoded > u64::from(self.limits.max_encoded_bytes) {
            return Err(ModelAdmissionError::new("encoded_limit"));
        }
        if self.decoded > u64::from(self.limits.max_decoded_bytes) {
            return Err(ModelAdmissionError::new("decoded_limit"));
        }
        if !is_digest(&payload.sha256) {
            return Err(ModelAdmissionError::new("invalid_digest"));
        }
        let length = payload.decoded_byte_length as usize;
        if payload.content.len() != length.div_ceil(3) * 4 {
            return Err(ModelAdmissionError::new("length_mismatch"));
        }
        // Allocate only the declared, already bounded length. A dishonest larger
        // decoding fails rather than growing the output buffer.
        let mut bytes = vec![0; length];
        let written = base64::engine::general_purpose::STANDARD
            .decode_slice(&payload.content, &mut bytes)
            .map_err(|_| ModelAdmissionError::new("invalid_base64"))?;
        if written != length {
            return Err(ModelAdmissionError::new("length_mismatch"));
        }
        let digest: String = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if digest != payload.sha256 {
            return Err(ModelAdmissionError::new("digest_mismatch"));
        }
        match payload.codec {
            ModelCodec::IonBinary => {
                #[cfg(not(target_arch = "wasm32"))]
                super::ion::inspect_ion(&bytes, profile, self.limits, &mut self.nodes)?;
                #[cfg(target_arch = "wasm32")]
                return Err(ModelAdmissionError::new("unsupported_codec"));
            }
            ModelCodec::MorphirJson => {
                super::json::inspect_json(&bytes, profile, self.limits, &mut self.nodes)?
            }
        }
        Ok(bytes)
    }

    pub(super) fn model(
        &mut self,
        model: &MorphirModel,
        tuple: &ModelTransportTuple,
    ) -> Result<RetainedModel, ModelAdmissionError> {
        if model.envelope_version != MODEL_ENVELOPE_VERSION
            || model.envelope_version != tuple.envelope_version
            || model.ir_version != tuple.ir_version
            || model.profile != tuple.profile
            || model.payload.codec != tuple.codec
        {
            return Err(ModelAdmissionError::new("unsupported_tuple"));
        }
        if model.model_id.trim().is_empty() || model.revision.trim().is_empty() {
            return Err(ModelAdmissionError::new("invalid_identity"));
        }
        const CRITICAL: &[&str] = &[
            "envelopeVersion",
            "modelId",
            "revision",
            "irVersion",
            "profile",
            "payload",
            "payload.codec",
            "payload.layout",
            "payload.decodedByteLength",
            "payload.sha256",
            "payload.content",
            "evidence",
            "critical",
        ];
        if let Some(path) = model
            .critical
            .iter()
            .find(|path| !CRITICAL.contains(&path.as_str()))
        {
            return Err(ModelAdmissionError::unknown_critical(path));
        }
        let bytes = self.payload(&model.payload, Some((&model.profile, &model.ir_version)))?;
        for evidence in &model.evidence {
            if [
                &evidence.guarantee,
                &evidence.model_id,
                &evidence.model_revision,
                &evidence.scope,
                &evidence.producer,
            ]
            .iter()
            .any(|value| value.trim().is_empty())
                || !is_digest(&evidence.context_digest)
            {
                return Err(ModelAdmissionError::new("invalid_evidence"));
            }
            self.payload(&evidence.evidence, None)?;
        }
        Ok(RetainedModel {
            envelope: model.clone(),
            bytes,
        })
    }
}

pub(super) fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Admit one model and all of its evidence under one aggregate payload budget.
///
/// Retains unknown Ion information as bytes; this does not trust the producer
/// or establish any typed/optimized guarantee.
pub fn retain_model(
    model: &MorphirModel,
    tuple: &ModelTransportTuple,
    limits: &ModelTransportLimits,
) -> Result<RetainedModel, ModelAdmissionError> {
    Budget::new(limits)?.model(model, tuple)
}

impl ModelPayload {
    /// Encode exact bytes with canonical base64, length and SHA-256.
    ///
    /// This builds a wire payload, without claiming that its codec/profile is
    /// admissible. The receiver still applies its configured aggregate limits.
    ///
    /// ```
    /// use morphir_extension_sdk::model::{ModelCodec, ModelPayload};
    /// let payload = ModelPayload::from_bytes(ModelCodec::MorphirJson, b"{}").unwrap();
    /// assert_eq!(payload.decoded_byte_length, 2);
    /// assert_eq!(payload.content, "e30=");
    /// ```
    pub fn from_bytes(codec: ModelCodec, bytes: &[u8]) -> Result<Self, ModelAdmissionError> {
        let length =
            u32::try_from(bytes.len()).map_err(|_| ModelAdmissionError::new("decoded_limit"))?;
        if length > i32::MAX as u32 {
            return Err(ModelAdmissionError::new("decoded_limit"));
        }
        Ok(Self {
            codec,
            layout: ModelLayout::SingleDocument,
            decoded_byte_length: length,
            sha256: Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            content: base64::engine::general_purpose::STANDARD.encode(bytes),
        })
    }
}

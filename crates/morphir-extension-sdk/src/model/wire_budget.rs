//! Check borrowed wire payload lengths before deserializing owned DTO strings.

use super::{ModelAdmissionError, ModelTransportLimits};
use serde_json::Value;

pub(super) struct WireBudget<'a> {
    limits: &'a ModelTransportLimits,
    encoded: u64,
    decoded: u64,
}

impl<'a> WireBudget<'a> {
    pub(super) fn new(limits: &'a ModelTransportLimits) -> Self {
        Self {
            limits,
            encoded: 0,
            decoded: 0,
        }
    }

    fn payload(&mut self, payload: &Value) -> Result<(), ModelAdmissionError> {
        let content = payload
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| ModelAdmissionError::new("invalid_params"))?;
        self.encoded = self.encoded.saturating_add(content.len() as u64);
        if self.encoded > u64::from(self.limits.max_encoded_bytes) {
            return Err(ModelAdmissionError::new("encoded_limit"));
        }
        let length = payload
            .get("decodedByteLength")
            .and_then(Value::as_u64)
            .ok_or_else(|| ModelAdmissionError::new("invalid_params"))?;
        self.decoded = self.decoded.saturating_add(length);
        if self.decoded > u64::from(self.limits.max_decoded_bytes) {
            return Err(ModelAdmissionError::new("decoded_limit"));
        }
        Ok(())
    }

    pub(super) fn model(&mut self, model: &Value) -> Result<(), ModelAdmissionError> {
        self.payload(&model["payload"])?;
        if let Some(evidence) = model.get("evidence") {
            for item in evidence
                .as_array()
                .ok_or_else(|| ModelAdmissionError::new("invalid_params"))?
            {
                self.payload(&item["evidence"])?;
            }
        }
        Ok(())
    }

    pub(super) fn dependencies(&mut self, params: &Value) -> Result<(), ModelAdmissionError> {
        if let Some(dependencies) = params.get("dependencies") {
            for dependency in dependencies
                .as_array()
                .ok_or_else(|| ModelAdmissionError::new("invalid_params"))?
            {
                self.model(&dependency["model"])?;
            }
        }
        Ok(())
    }
}

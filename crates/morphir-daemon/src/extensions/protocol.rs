//! Shared Morphir Extension Protocol types and host-side response helpers.

pub use morphir_extension_sdk::protocol::*;

/// Maximum request or response payload accepted by built-in MEP transports.
pub const MAX_MEP_PAYLOAD_BYTES: u32 = 64 * 1024 * 1024;

/// Host-side conversion from a JSON-RPC response to its typed result.
pub trait ExtensionResponseExt {
    /// Validate the JSON-RPC envelope without interpreting its payload.
    fn validate_envelope(&self, expected_id: u64) -> crate::Result<()>;

    /// Deserialize a successful result or return the extension error.
    fn into_result<T: serde::de::DeserializeOwned>(self, expected_id: u64) -> crate::Result<T>;
}

impl ExtensionResponseExt for ExtensionResponse {
    fn validate_envelope(&self, expected_id: u64) -> crate::Result<()> {
        if self.jsonrpc != JSONRPC_VERSION {
            return Err(crate::DaemonError::Extension(format!(
                "Extension response used unsupported JSON-RPC version '{}'",
                self.jsonrpc
            )));
        }
        if self.id != expected_id {
            return Err(crate::DaemonError::Extension(format!(
                "Extension response ID {} did not match request ID {expected_id}",
                self.id
            )));
        }
        if self.result.is_some() == self.error.is_some() {
            return Err(crate::DaemonError::Extension(
                "Extension response must contain exactly one of result or error".to_string(),
            ));
        }
        Ok(())
    }

    fn into_result<T: serde::de::DeserializeOwned>(self, expected_id: u64) -> crate::Result<T> {
        self.validate_envelope(expected_id)?;

        match (self.result, self.error) {
            (Some(value), None) => serde_json::from_value(value).map_err(crate::DaemonError::from),
            (None, Some(error)) => Err(crate::DaemonError::Extension(format!(
                "RPC error {}: {}",
                error.code, error.message
            ))),
            _ => unreachable!("the response envelope was validated"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_response_with_both_result_and_error() {
        let response = ExtensionResponse {
            jsonrpc: JSONRPC_VERSION.into(),
            result: Some(serde_json::json!({})),
            error: Some(RpcError::internal_error("broken")),
            id: 1,
        };
        let error = response.into_result::<serde_json::Value>(1).unwrap_err();
        assert!(error.to_string().contains("exactly one"));
    }

    #[test]
    fn rejects_a_response_for_another_request() {
        let response = ExtensionResponse::success(2, serde_json::json!({})).unwrap();
        let error = response.into_result::<serde_json::Value>(1).unwrap_err();
        assert!(error.to_string().contains("did not match"));
    }
}

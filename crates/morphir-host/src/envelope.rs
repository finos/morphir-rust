//! Validation of an untrusted JSON-RPC response envelope.

use crate::HostError;
use morphir_extension_sdk::protocol::{ExtensionResponse, JSONRPC_VERSION};

/// Why a response envelope did not yield a result.
#[derive(Debug)]
pub enum EnvelopeError {
    /// The guest answered with a JSON-RPC error. The session is still sound.
    Rpc(HostError),
    /// The envelope broke the protocol. The session cannot be trusted.
    Invalid(HostError),
}

/// Check the envelope of the response to request `expected_id`.
pub fn validate_envelope(
    response: ExtensionResponse,
    expected_id: u64,
) -> Result<serde_json::Value, EnvelopeError> {
    if response.jsonrpc != JSONRPC_VERSION {
        return Err(EnvelopeError::Invalid(HostError::Invalid(format!(
            "Extension response used unsupported JSON-RPC version '{}'",
            response.jsonrpc
        ))));
    }
    if response.id != expected_id {
        return Err(EnvelopeError::Invalid(HostError::Invalid(format!(
            "Extension response ID {} did not match request ID {expected_id}",
            response.id
        ))));
    }
    match (response.result, response.error) {
        (Some(value), None) => Ok(value),
        (None, Some(error)) => Err(EnvelopeError::Rpc(HostError::Rpc(error))),
        _ => Err(EnvelopeError::Invalid(HostError::Invalid(
            "Extension response must contain exactly one of result or error".into(),
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_extension_sdk::protocol::{ExtensionResponse, RpcError};

    fn invalid_message(outcome: Result<serde_json::Value, EnvelopeError>) -> String {
        match outcome {
            Err(EnvelopeError::Invalid(error)) => error.to_string(),
            other => panic!("expected an invalid envelope, got {other:?}"),
        }
    }

    #[test]
    fn a_result_with_the_matching_id_is_accepted() {
        let response = ExtensionResponse::success(7, serde_json::json!({"ok": true})).unwrap();
        assert_eq!(
            validate_envelope(response, 7).unwrap(),
            serde_json::json!({"ok": true})
        );
    }

    #[test]
    fn a_json_rpc_error_is_reported_with_its_code() {
        let response = ExtensionResponse::error(
            3,
            RpcError {
                code: -32013,
                message: "no".into(),
                data: None,
            },
        );
        match validate_envelope(response, 3) {
            Err(EnvelopeError::Rpc(error)) => assert_eq!(error.to_string(), "RPC error -32013: no"),
            other => panic!("expected an RPC error, got {other:?}"),
        }
    }

    #[test]
    fn a_mismatched_id_is_invalid() {
        let response = ExtensionResponse::success(9, serde_json::json!({})).unwrap();
        assert_eq!(
            invalid_message(validate_envelope(response, 2)),
            "Extension response ID 9 did not match request ID 2"
        );
    }

    #[test]
    fn a_foreign_json_rpc_version_is_invalid() {
        let mut response = ExtensionResponse::success(1, serde_json::json!({})).unwrap();
        response.jsonrpc = "1.0".into();
        assert_eq!(
            invalid_message(validate_envelope(response, 1)),
            "Extension response used unsupported JSON-RPC version '1.0'"
        );
    }

    #[test]
    fn a_response_with_neither_result_nor_error_is_invalid() {
        let mut response = ExtensionResponse::success(1, serde_json::json!({})).unwrap();
        response.result = None;
        assert_eq!(
            invalid_message(validate_envelope(response, 1)),
            "Extension response must contain exactly one of result or error"
        );
    }
}

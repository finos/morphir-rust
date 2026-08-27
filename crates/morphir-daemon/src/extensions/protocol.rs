//! Shared Morphir Extension Protocol types and host-side response helpers.

pub use morphir_extension_sdk::protocol::*;

/// Maximum request or response payload accepted by built-in MEP transports.
pub const MAX_MEP_PAYLOAD_BYTES: u32 = 64 * 1024 * 1024;

/// Host-side conversion from a JSON-RPC response to its typed result.
pub trait ExtensionResponseExt {
    /// Deserialize a successful result or return the extension error.
    fn into_result<T: serde::de::DeserializeOwned>(self) -> crate::Result<T>;
}

impl ExtensionResponseExt for ExtensionResponse {
    fn into_result<T: serde::de::DeserializeOwned>(self) -> crate::Result<T> {
        if let Some(error) = self.error {
            return Err(crate::DaemonError::Extension(format!(
                "RPC error {}: {}",
                error.code, error.message
            )));
        }

        match self.result {
            Some(value) => serde_json::from_value(value).map_err(crate::DaemonError::from),
            None => Err(crate::DaemonError::Extension(
                "Empty response from extension".to_string(),
            )),
        }
    }
}

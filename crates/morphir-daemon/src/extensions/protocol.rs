//! JSON-RPC 2.0 protocol types for extension communication
//!
//! This module re-exports protocol types from the SDK and adds host-specific utilities.

use serde::{Deserialize, Serialize};

/// JSON-RPC version string
pub const JSONRPC_VERSION: &str = "2.0";

/// Standard extension method names
pub mod methods {
    /// Get extension info
    pub const INFO: &str = "morphir.extension.info";
    /// Get extension capabilities
    pub const CAPABILITIES: &str = "morphir.extension.capabilities";
    /// Frontend: compile source to IR
    pub const COMPILE: &str = "morphir.frontend.compile";
    /// Backend: generate code from IR
    pub const GENERATE: &str = "morphir.backend.generate";
    /// Validator: validate IR
    pub const VALIDATE: &str = "morphir.validator.validate";
    /// Transform: transform IR to IR
    pub const TRANSFORM: &str = "morphir.transform.transform";
}

/// JSON-RPC 2.0 Request to extension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionRequest {
    /// JSON-RPC version (always "2.0")
    pub jsonrpc: String,
    /// Method name
    pub method: String,
    /// Method parameters
    pub params: serde_json::Value,
    /// Request ID
    pub id: u64,
}

impl ExtensionRequest {
    /// Create a new request
    pub fn new<P: Serialize>(method: &str, params: P, id: u64) -> Result<Self, serde_json::Error> {
        Ok(Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            method: method.to_string(),
            params: serde_json::to_value(params)?,
            id,
        })
    }
}

/// JSON-RPC 2.0 Response from extension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionResponse {
    /// JSON-RPC version (always "2.0")
    pub jsonrpc: String,
    /// Result (mutually exclusive with error)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Error (mutually exclusive with result)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
    /// Request ID this is responding to
    pub id: u64,
}

impl ExtensionResponse {
    /// Check if the response indicates success
    pub fn is_success(&self) -> bool {
        self.error.is_none() && self.result.is_some()
    }

    /// Get the result, returning an error if the response was an error
    pub fn into_result<T: serde::de::DeserializeOwned>(self) -> Result<T, crate::DaemonError> {
        if let Some(err) = self.error {
            return Err(crate::DaemonError::Extension(format!(
                "RPC error {}: {}",
                err.code, err.message
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

/// JSON-RPC 2.0 Error object
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    /// Error code
    pub code: i32,
    /// Error message
    pub message: String,
    /// Additional error data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Standard JSON-RPC 2.0 error codes
pub mod error_codes {
    /// Parse error - Invalid JSON
    pub const PARSE_ERROR: i32 = -32700;
    /// Invalid request - Not a valid JSON-RPC request
    pub const INVALID_REQUEST: i32 = -32600;
    /// Method not found
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// Invalid params
    pub const INVALID_PARAMS: i32 = -32602;
    /// Internal error
    pub const INTERNAL_ERROR: i32 = -32603;

    // Custom error codes (server-defined: -32000 to -32099)
    /// Extension error
    pub const EXTENSION_ERROR: i32 = -32000;
    /// Compilation error
    pub const COMPILATION_ERROR: i32 = -32001;
    /// Generation error
    pub const GENERATION_ERROR: i32 = -32002;
    /// Validation error
    pub const VALIDATION_ERROR: i32 = -32003;
    /// Transformation error
    pub const TRANSFORMATION_ERROR: i32 = -32004;
}

impl RpcError {
    /// Create a method not found error
    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: error_codes::METHOD_NOT_FOUND,
            message: format!("Method not found: {}", method),
            data: None,
        }
    }

    /// Create an internal error
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self {
            code: error_codes::INTERNAL_ERROR,
            message: message.into(),
            data: None,
        }
    }
}

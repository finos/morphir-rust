//! JSON-RPC 2.0 protocol types for extension communication
//!
//! Extensions communicate with the host via JSON-RPC 2.0 payloads.

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
    /// Create a success response
    pub fn success<R: Serialize>(id: u64, result: R) -> Result<Self, serde_json::Error> {
        Ok(Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            result: Some(serde_json::to_value(result)?),
            error: None,
            id,
        })
    }

    /// Create an error response
    pub fn error(id: u64, error: RpcError) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            result: None,
            error: Some(error),
            id,
        }
    }

    /// Create an error response from an extension error
    pub fn from_extension_error(id: u64, err: &crate::ExtensionError) -> Self {
        Self::error(id, RpcError::from_extension_error(err))
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
    /// Create a parse error
    pub fn parse_error(message: impl Into<String>) -> Self {
        Self {
            code: error_codes::PARSE_ERROR,
            message: message.into(),
            data: None,
        }
    }

    /// Create an invalid request error
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: error_codes::INVALID_REQUEST,
            message: message.into(),
            data: None,
        }
    }

    /// Create a method not found error
    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: error_codes::METHOD_NOT_FOUND,
            message: format!("Method not found: {}", method),
            data: None,
        }
    }

    /// Create an invalid params error
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: error_codes::INVALID_PARAMS,
            message: message.into(),
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

    /// Create an extension error
    pub fn extension_error(message: impl Into<String>) -> Self {
        Self {
            code: error_codes::EXTENSION_ERROR,
            message: message.into(),
            data: None,
        }
    }

    /// Create an error from an ExtensionError
    pub fn from_extension_error(err: &crate::ExtensionError) -> Self {
        use crate::ExtensionError;

        let (code, message) = match err {
            ExtensionError::NotFound(id) => {
                (error_codes::METHOD_NOT_FOUND, format!("Extension not found: {}", id))
            }
            ExtensionError::LoadFailed(msg) => (error_codes::EXTENSION_ERROR, msg.clone()),
            ExtensionError::InitFailed(msg) => (error_codes::EXTENSION_ERROR, msg.clone()),
            ExtensionError::UnsupportedCapability { extension, capability } => (
                error_codes::EXTENSION_ERROR,
                format!("Extension '{}' does not support: {}", extension, capability),
            ),
            ExtensionError::ExecutionFailed(msg) => (error_codes::INTERNAL_ERROR, msg.clone()),
            ExtensionError::InvalidResponse(msg) => (error_codes::INTERNAL_ERROR, msg.clone()),
            ExtensionError::Io(e) => (error_codes::INTERNAL_ERROR, e.to_string()),
            ExtensionError::Json(e) => (error_codes::PARSE_ERROR, e.to_string()),
        };

        Self {
            code,
            message,
            data: None,
        }
    }
}

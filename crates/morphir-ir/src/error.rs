//! Error types for Morphir models
//!
//! This module provides error handling following functional programming principles
//! with clear error types and composable error handling.

use thiserror::Error;

/// Result type alias for Morphir operations
pub type Result<T> = std::result::Result<T, Error>;

/// Main error type for Morphir model operations
#[derive(Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid Morphir IR: {0}")]
    InvalidIr(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Not implemented: {0}")]
    NotImplemented(String),
}

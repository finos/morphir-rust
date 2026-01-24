//! Morphir CLI Library
//!
//! This library exposes CLI functionality for programmatic use and testing.

pub mod output;
pub mod error;
pub mod commands;

pub use output::OutputFormat;
pub use error::CliError;

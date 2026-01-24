//! Morphir CLI Library
//!
//! This library exposes CLI functionality for programmatic use and testing.

pub mod commands;
pub mod error;
pub mod output;

pub use error::CliError;
pub use output::OutputFormat;

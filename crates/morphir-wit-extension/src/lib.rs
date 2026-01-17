//! Morphir WIT Extension - WASM Component Model runtime for Morphir extensions
//!
//! This crate provides the infrastructure for loading and running Morphir extensions
//! as WASM components using the WebAssembly Component Model and WIT interfaces.
//!
//! # Extension Types
//!
//! Extensions can implement one or more capabilities:
//! - **Frontend**: Parse source languages into Morphir IR
//! - **Backend**: Generate code from Morphir IR
//! - **Transform**: Transform IR to IR
//! - **Analyzer**: Analyze IR and produce diagnostics
//!
//! # Architecture
//!
//! Extensions are loaded as WASM components and communicate with the host via
//! WIT-defined interfaces. The runtime manages extension lifecycle, capability
//! negotiation, and resource limits.

pub mod error;
pub mod runtime;
pub mod types;

pub use error::{ExtensionError, Result};

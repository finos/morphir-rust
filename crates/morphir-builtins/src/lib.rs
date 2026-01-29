//! Morphir Builtin Extensions
//!
//! This crate provides builtin extensions that ship with Morphir.
//! Each builtin supports:
//! - **Native execution**: Direct Rust implementation (always available)
//! - **WASM mode**: Can be compiled to WASM for extension architecture testing
//!
//! # Builtins
//!
//! - `migrate`: IR version migration (v3 ↔ v4)
//!
//! # Usage
//!
//! ## Native Mode (Direct)
//!
//! ```rust,ignore
//! use morphir_builtins::migrate::MigrateExtension;
//! use morphir_ext_core::Envelope;
//!
//! let migrate = MigrateExtension::default();
//! let result = migrate.execute_native(&input_envelope)?;
//! ```
//!
//! ## WASM Mode (Extension Architecture)
//!
//! ```rust,ignore
//! use morphir_builtins::migrate::MigrateExtension;
//! use morphir_ext::{DirectRuntime, ExtismRuntime};
//!
//! // Get embedded WASM bytes
//! let wasm_bytes = MigrateExtension::wasm_bytes()
//!     .expect("WASM feature not enabled");
//!
//! // Load via extension runtime
//! let runtime = ExtismRuntime::new(wasm_bytes.to_vec())?;
//! let direct = DirectRuntime::new(Box::new(runtime));
//! let result = direct.execute("backend_generate", &input_envelope)?;
//! ```

use anyhow::Result;
use morphir_ext_core::Envelope;

#[cfg(feature = "migrate")]
pub mod migrate;

pub mod registry;

/// Common trait for all builtin extensions.
///
/// Builtins implement this trait to provide both native and WASM execution modes.
pub trait BuiltinExtension: Send + Sync {
    /// Execute the extension using native Rust implementation.
    ///
    /// This is always available and provides the best performance.
    fn execute_native(&self, input: &Envelope) -> Result<Envelope>;

    /// Get the extension's metadata.
    fn info(&self) -> BuiltinInfo;

    /// Get embedded WASM bytes (if compiled with `wasm` feature).
    ///
    /// Returns `Some` when the extension has been compiled to WASM and
    /// embedded in the binary via `include_bytes!`.
    #[cfg(feature = "wasm")]
    fn wasm_bytes() -> Option<&'static [u8]> {
        None
    }
}

/// Metadata about a builtin extension.
#[derive(Debug, Clone)]
pub struct BuiltinInfo {
    /// Unique identifier (e.g., "migrate")
    pub id: String,
    /// Display name
    pub name: String,
    /// Extension type (frontend, backend, transform)
    pub extension_type: ExtensionType,
    /// Description
    pub description: String,
}

/// Type of builtin extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionType {
    /// Frontend: source → IR
    Frontend,
    /// Backend: IR → target code
    Backend,
    /// Transform: IR → IR
    Transform,
    /// Validator: IR → diagnostics
    Validator,
}

impl std::fmt::Display for ExtensionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtensionType::Frontend => write!(f, "frontend"),
            ExtensionType::Backend => write!(f, "backend"),
            ExtensionType::Transform => write!(f, "transform"),
            ExtensionType::Validator => write!(f, "validator"),
        }
    }
}

//! Extension system for morphir-daemon
//!
//! This module provides the Extism-based plugin runtime for loading
//! and executing Morphir extensions.

pub mod container;
pub mod host_functions;
pub mod loader;
pub mod protocol;
pub mod registry;
pub mod virtual_paths;

pub use container::ExtensionContainer;
pub use loader::ExtensionLoader;
pub use protocol::{ExtensionRequest, ExtensionResponse};
pub use registry::ExtensionRegistry;

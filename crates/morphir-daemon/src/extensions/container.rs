//! The Extism extension container.
//!
//! The container moved to `morphir_host_native::extism`. This module keeps
//! the old path resolving for callers inside and outside this crate.

pub use morphir_extension_sdk::{ExtensionInfo, ExtensionType};
pub use morphir_host_native::extism::{ExtensionContainer, ExtensionContainerBuilder};

//! Morphir Daemon - Long-running service for workspace management
//!
//! This crate provides the Morphir daemon service which handles:
//! - Workspace lifecycle management (create, open, close)
//! - Project management within workspaces
//! - Dependency resolution and caching
//! - Incremental builds with file watching
//! - JSON-RPC protocol for CLI and IDE integration
//! - Extension loading and management via Extism

pub mod error;
pub mod extensions;
pub mod workspace;

pub use error::{DaemonError, Result};
pub use extensions::{ExtensionContainer, ExtensionLoader, ExtensionRegistry};

//! Extension files and file access for morphir-daemon.
//!
//! The daemon does not run extensions itself. Resolution, activation and
//! sessions live in `morphir-host` and `morphir-host-native`. This module
//! keeps what is specific to the daemon: fetching and caching extension
//! files, and mapping the virtual paths an extension sees to real paths.

pub mod loader;
pub mod virtual_paths;

pub use loader::ExtensionLoader;
pub use virtual_paths::{FileSandbox, SandboxError, VirtualPathConfig};

//! Morphir daemon services that sit on top of the extension host.
//!
//! This crate is a client of `morphir-host`. It has no MEP codec, handshake,
//! extension registry or session of its own. To resolve, start and call an
//! extension, use `morphir_host::{Registry, Session, Pool}` with the channels
//! and `activate` in `morphir-host-native`.
//!
//! What this crate provides:
//! - [`DaemonError`], the error type the CLI reports, with a conversion from
//!   `morphir_host::HostError` that keeps the host's error texts.
//! - [`ExtensionLoader`], which fetches extension files from a path, a URL or
//!   a GitHub release and caches them under the Morphir home.
//! - [`VirtualPathConfig`] and [`FileSandbox`], which map the virtual paths an
//!   extension sees to real paths and limit what it can read and write.
//! - [`workspace`], the workspace and project model built from workspace
//!   discovery.

pub mod error;
pub mod extensions;
pub mod workspace;

pub use error::{DaemonError, Result};
pub use extensions::{ExtensionLoader, FileSandbox, SandboxError, VirtualPathConfig};

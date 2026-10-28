//! The Extism plugin runtime as a MEP channel.
//!
//! This module holds the container that loads and calls a WASM guest, the
//! host functions the guest can call back into, and the channel that carries
//! MEP messages over a loaded container.
//!
//! Hosts with already verified guest bytes can opt in to [`RestrictedExtismPolicy`].
//! It denies all HTTP hosts, WASI preopened paths and inherited guest environment,
//! and can only tighten legacy memory, fuel, timeout and encoded message limits.
//! The response bound applies to discovery and borrowed guest output before a
//! host-owned copy. Request serialization stops at the configured byte boundary.
//! Existing constructors and the separate generated-artifact profile are unchanged.
//! WASI remains enabled for guest runtime support. Callers must sanitize their own
//! host environment, including Extism's optional `EXTISM_ENABLE_WASI_OUTPUT`, and
//! retain whole-session deadlines and process isolation for hard cancellation.
//!
//! ```no_run
//! use morphir_host_native::extism::{
//!     ExtensionContainer, MorphirHostFunctions, RestrictedExtismPolicy,
//! };
//! use std::time::Duration;
//! # async fn load(verified_wasm: Vec<u8>) -> Result<(), morphir_host::HostError> {
//! let policy = RestrictedExtismPolicy::new(
//!     256 * 1024 * 1024, 1_000_000_000, Duration::from_secs(30),
//!     4 * 1024 * 1024, 16 * 1024 * 1024,
//! )?;
//! let container = ExtensionContainer::from_bytes_with_execution_policy_async(
//!     "morphir-python".into(), verified_wasm, MorphirHostFunctions::default(), policy,
//! ).await?;
//! assert_eq!(container.info().id, "morphir-python");
//! # Ok(())
//! # }
//! ```

mod channel;
mod container;
mod host_functions;
mod policy;

pub use channel::ExtismChannel;
pub use container::{ExtensionContainer, ExtensionContainerBuilder};
pub use host_functions::{MorphirHostFunctions, MorphirHostState};
pub use policy::RestrictedExtismPolicy;

//! Native channels for the Morphir extension host.
//!
//! `morphir-host` is portable and has no native dependencies. This crate
//! holds the parts that need a native runtime: child processes, Extism, and
//! in-process Rust guests, each behind `morphir_host::Channel`. It also
//! checks guest results before the host trusts them, and starts installed
//! guests from their verified artifacts.

mod activate;
pub mod extism;
mod native;
pub mod process;
mod results;

pub use activate::{ActivatedGuest, activate};
pub use native::NativeChannel;
pub use results::{CheckedConnection, validate_result};

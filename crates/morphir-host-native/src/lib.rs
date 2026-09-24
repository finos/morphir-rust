//! Native channels for the Morphir extension host.
//!
//! `morphir-host` is portable and has no native dependencies. This crate
//! holds the parts that need a native runtime: child processes, Extism, and
//! in-process Rust guests, each behind `morphir_host::Channel`.

pub mod extism;
mod native;
pub mod process;

pub use native::NativeChannel;

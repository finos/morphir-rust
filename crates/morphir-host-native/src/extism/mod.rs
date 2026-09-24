//! The Extism plugin runtime as a MEP channel.
//!
//! This module holds the container that loads and calls a WASM guest, the
//! host functions the guest can call back into, and the channel that carries
//! MEP messages over a loaded container.

mod channel;
mod container;
mod host_functions;

pub use channel::ExtismChannel;
pub use container::{ExtensionContainer, ExtensionContainerBuilder};
pub use host_functions::{MorphirHostFunctions, MorphirHostState};

//! Host functions exposed to extension plugins.
//!
//! These moved to `morphir_host_native::extism`. This module keeps the old
//! path resolving for callers inside and outside this crate.

pub use morphir_host_native::extism::{MorphirHostFunctions, MorphirHostState};

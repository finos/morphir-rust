//! WASM backend - generate WebAssembly from Morphir IR

mod codegen;
mod wat;

pub use codegen::generate_wasm;
pub use wat::generate_wat;

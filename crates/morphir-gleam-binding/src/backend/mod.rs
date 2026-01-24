//! Gleam backend - generate Gleam code from Morphir IR

mod codegen;
mod visitor;

pub use codegen::generate_gleam;
pub use visitor::MorphirToGleamVisitor;

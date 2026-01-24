//! Gleam backend - generate Gleam code from Morphir IR

pub mod codegen;
pub mod visitor;

pub use codegen::generate_gleam;
pub use visitor::MorphirToGleamVisitor;

//! Gleam frontend - parse Gleam source to Morphir IR

pub mod ast;
pub mod errors;
pub mod lexer;
pub mod parser;
pub mod visitor;

pub use parser::parse_gleam;
pub use visitor::{DistributionLayout, GleamToMorphirVisitor};

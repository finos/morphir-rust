//! Gleam frontend - parse Gleam source to Morphir IR

mod parser;
mod visitor;

pub use parser::parse_gleam;
pub use visitor::{GleamToMorphirVisitor, DistributionLayout};
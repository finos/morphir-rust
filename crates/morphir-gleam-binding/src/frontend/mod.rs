//! Gleam frontend - parse Gleam source to Morphir IR

pub mod parser;
pub mod visitor;

pub use parser::parse_gleam;
pub use visitor::{DistributionLayout, GleamToMorphirVisitor};

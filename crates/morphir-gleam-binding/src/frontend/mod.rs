//! Gleam frontend - parse Gleam source to Morphir IR

pub mod ast;
pub mod compare;
pub mod errors;
pub mod lexer;
pub mod parser;
pub mod visitor;

pub use compare::{ComparisonResult, Difference, compare_modules, modules_equivalent};
pub use parser::parse_gleam;
pub use visitor::{DistributionLayout, GleamToMorphirVisitor};

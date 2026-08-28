//! Gleam frontend - parse Gleam source to Morphir IR

pub mod ast;
pub mod compare;
pub mod errors;
pub mod lexer;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod parse_stage;
#[cfg(target_arch = "wasm32")]
#[path = "parse_stage_wasm.rs"]
pub(crate) mod parse_stage;
pub mod parser;
pub mod visitor;

pub use compare::{ComparisonResult, Difference, compare_modules, modules_equivalent};
pub use parser::parse_gleam;
pub use visitor::{DistributionLayout, GleamToMorphirVisitor};

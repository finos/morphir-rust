//! The Elm frontend: Elm source in, a Morphir IR distribution and one result
//! per module out.
//!
//! The pipeline is [`parse`] → [`cst_to_ast`] → [`resolve`] → [`emit`], driven
//! by [`compile`]; [`boundary`] checks the request before any of it runs and
//! [`dependencies`] reads what the request supplied to resolve against.

pub mod boundary;
pub mod compile;
pub mod cst_to_ast;
pub mod dependencies;
pub mod emit;
pub mod parse;
pub mod resolve;
pub mod source;

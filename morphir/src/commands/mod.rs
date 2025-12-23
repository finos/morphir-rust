//! Command modules for Morphir CLI
//!
//! This module contains all command implementations following functional
//! domain modeling principles.

pub mod generate;
pub mod transform;
pub mod validate;

pub use generate::run_generate;
pub use transform::run_transform;
pub use validate::run_validate;


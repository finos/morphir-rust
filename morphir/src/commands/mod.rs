//! Command modules for Morphir CLI
//!
//! This module contains all command implementations following functional
//! domain modeling principles.

pub mod generate;
pub mod tool;
pub mod transform;
pub mod validate;

pub use generate::run_generate;
pub use tool::{run_tool_install, run_tool_list, run_tool_uninstall, run_tool_update};
pub use transform::run_transform;
pub use validate::run_validate;


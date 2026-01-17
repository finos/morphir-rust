//! Command modules for Morphir CLI
//!
//! This module contains all command implementations following functional
//! domain modeling principles.

pub mod dist;
pub mod extension;
pub mod generate;
pub mod tool;
pub mod transform;
pub mod validate;

pub use dist::{run_dist_install, run_dist_list, run_dist_uninstall, run_dist_update};
pub use extension::{run_extension_install, run_extension_list, run_extension_uninstall, run_extension_update};
pub use generate::run_generate;
pub use tool::{run_tool_install, run_tool_list, run_tool_uninstall, run_tool_update};
pub use transform::run_transform;
pub use validate::run_validate;


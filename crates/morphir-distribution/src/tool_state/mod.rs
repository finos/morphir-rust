//! Exact tool locks, active catalog state, and offline launch verification.

mod activation;
mod catalog;
mod package;
mod package_key;
mod raw_package;
mod recovery;
mod repair_journal;
mod verification;

pub use activation::{VerifiedToolProcess, activate_installed_tool};
pub use catalog::{
    InstalledTool, InstalledToolSnapshot, ToolInstaller, ToolLock, list_installed_tools,
    read_tool_lock,
};
pub use package::{ToolPackageStore, VerifiedToolPackage};
pub use recovery::{ToolRepairer, rollback_tool};

#[cfg(test)]
use recovery::rollback_with_writer;

#[cfg(test)]
mod tests;

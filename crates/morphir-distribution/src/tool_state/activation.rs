//! Network-free activation of installed tools.

use super::catalog::{
    load_catalog_unlocked, read_tool_lock_unlocked, tool_state_guard, validate_active_pair,
};
use super::verification::verify_installed;
use crate::{DistributionError, Result, ToolId};
use morphir_common::home::MorphirHome;
use semver::Version;
use std::path::{Path, PathBuf};

/// Offline launch contract whose active program bytes have just been reverified.
#[derive(Debug, Clone)]
pub struct VerifiedToolProcess {
    program: PathBuf,
    args: Vec<String>,
    tool_id: ToolId,
    version: Version,
}

impl VerifiedToolProcess {
    /// Return the verified absolute executable path.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Return fixed arguments prepended during launch.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Return the launched tool identity.
    pub fn tool_id(&self) -> &ToolId {
        &self.tool_id
    }

    /// Return the launched exact semantic version.
    pub fn version(&self) -> &Version {
        &self.version
    }
}

/// Resolve one active catalog entry without repository or network access and reverify its bytes.
#[tracing::instrument(
    name = "morphir.tool.activate",
    skip(home),
    fields(tool_id = %id),
    err
)]
pub fn activate_installed_tool(home: &MorphirHome, id: &ToolId) -> Result<VerifiedToolProcess> {
    let _transaction = tool_state_guard(home)?;
    let tools = load_catalog_unlocked(home)?;
    let active = tools
        .get(id)
        .map(|entry| entry.active.clone())
        .ok_or_else(|| DistributionError::ToolNotInstalled { id: id.clone() })?;
    let lock = read_tool_lock_unlocked(home, id)?;
    validate_active_pair(&active, &lock)?;
    let program = verify_installed(home, active.store_path.as_path(), &active.files)?;
    tracing::info!(
        tool_id = %active.tool_id,
        version = %active.version,
        program = %program.display(),
        "installed tool verified for offline launch"
    );
    Ok(VerifiedToolProcess {
        program,
        args: active.args,
        tool_id: active.tool_id,
        version: active.version,
    })
}

//! Network-free activation of installed tools.

use super::catalog::{
    load_catalog_unlocked, read_tool_lock_unlocked, tool_state_guard, validate_active_pair,
};
use super::verification::verify_installed;
use crate::state_io::StateGuard;
use crate::{DistributionError, RelativeArtifactPath, Result, ToolId};
use morphir_common::home::MorphirHome;
use semver::Version;
use std::path::{Path, PathBuf};

/// Offline launch contract whose active program bytes have just been reverified.
///
/// The contract retains the tool-state guard so an installer or repair cannot replace the
/// verified program between activation and process creation. Keep it alive until the child has
/// been spawned, then drop it to allow other tool-state operations to continue.
#[derive(Debug)]
pub struct VerifiedToolProcess {
    program: PathBuf,
    args: Vec<String>,
    tool_id: ToolId,
    version: Version,
    _state_guard: StateGuard,
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
///
/// The returned contract serializes tool-state changes until it is dropped. Callers should keep
/// the contract alive while spawning the verified executable.
#[tracing::instrument(
    name = "morphir.tool.activate",
    skip(home),
    fields(tool_id = %id),
    err
)]
pub fn activate_installed_tool(home: &MorphirHome, id: &ToolId) -> Result<VerifiedToolProcess> {
    let state_guard = tool_state_guard(home)?;
    let tools = load_catalog_unlocked(home)?;
    let entry = tools
        .get(id)
        .ok_or_else(|| DistributionError::ToolNotInstalled { id: id.clone() })?;
    let lock = read_tool_lock_unlocked(home, id)?;
    validate_active_pair(&entry.active, &entry.rollback, &lock)?;
    let active = entry.active.clone();
    let program = verify_installed(
        home,
        active.store_path.as_path(),
        active
            .package_root
            .as_ref()
            .map(RelativeArtifactPath::as_path),
        &active.files,
        &active.directories,
    )?;
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
        _state_guard: state_guard,
    })
}

//! Discovery rooted in a directory capability already owned by the host.
use anyhow::{Context, anyhow, bail};
use cap_std::fs::Dir;
use morphir_common::file_identity::FileIdentity;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// A provider-owned directory capability bound to verified host-selected labels.
///
/// The directory remains the authority for development-tree reads. Root labels
/// support diagnostics and the existing confined absolute-alias policy. They
/// never grant access to a replacement directory. Construction checks the
/// label's canonical target against the supplied directory. Each capture verifies
/// both labels again before reading configuration files.
///
/// ```no_run
/// use cap_std::{ambient_authority, fs::Dir};
/// use morphir_devkit::{ConfigLoadOptions, WorkspaceDiscoveryRoot};
/// use std::path::Path;
///
/// # fn main() -> anyhow::Result<()> {
/// let label = Path::new(".");
/// // The host grants this directory once, then passes its existing capability.
/// let directory = Dir::open_ambient_dir(label, ambient_authority())?;
/// let root = WorkspaceDiscoveryRoot::from_directory(directory, label)?;
/// let captured = root.capture(&ConfigLoadOptions::project_only())?;
/// let details = captured.into_typed_details()?;
/// assert!(!details.canonical_root.as_os_str().is_empty());
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct WorkspaceDiscoveryRoot {
    directory: Dir,
    granted_root: PathBuf,
    canonical_root: PathBuf,
    identity: FileIdentity,
}
impl WorkspaceDiscoveryRoot {
    /// Capture the same confined discovery request while observing real reads.
    /// Selected mounts outside the workspace count as ambient even when allowed
    /// by the options. A sealed scope covers only this synchronous SDK prefix.
    ///
    /// ```no_run
    /// use cap_std::{ambient_authority, fs::Dir};
    /// use morphir_common::config::observation::{ReadLimits, SourceReadScope};
    /// use morphir_devkit::{ConfigLoadOptions, WorkspaceDiscoveryRoot};
    /// use std::path::Path;
    /// # fn main() -> anyhow::Result<()> {
    /// let path = Path::new("/already-authorized/workspace");
    /// let held = Dir::open_ambient_dir(path, ambient_authority())?;
    /// let scope = SourceReadScope::bind_workspace(&held, path, ReadLimits::new(16, 128, 1_048_576, 1)?)?;
    /// let root = WorkspaceDiscoveryRoot::from_directory(held, path)?;
    /// let capture = root.capture_observed(&ConfigLoadOptions::project_only(), &scope);
    /// scope.seal().require_complete()?;
    /// let request = capture?;
    /// # let _ = request;
    /// # Ok(()) }
    /// ```
    pub fn capture_observed(
        &self,
        options: &super::ConfigLoadOptions,
        scope: &morphir_common::config::observation::SourceReadScope,
    ) -> anyhow::Result<CapturedWorkspaceDiscovery> {
        self.verify_current()?;
        let request = super::capture_request_with_reads(
            &self.directory,
            &self.canonical_root,
            &self.granted_root,
            options,
            super::AliasBudgets::DEFAULT,
            super::TraversalBudgets::DEFAULT,
            Some(scope),
        )?;
        Ok(CapturedWorkspaceDiscovery {
            canonical_root: self.canonical_root.clone(),
            request,
        })
    }
    /// Bind an existing directory to the host-selected root label.
    ///
    /// Only directory identity metadata is read here. The label supplies no
    /// configuration authority; subsequent traversal uses the supplied Dir.
    pub fn from_directory(directory: Dir, host_root: &Path) -> anyhow::Result<Self> {
        let granted_root = if host_root.is_absolute() {
            host_root.to_owned()
        } else {
            std::env::current_dir()
                .context("Failed to resolve relative development root")?
                .join(host_root)
        };
        let canonical_root = fs::canonicalize(&granted_root).map_err(|error| {
            anyhow!(
                "workspace.path.not-confined: development root changed after binding `{}`: {error}",
                granted_root.display()
            )
        })?;
        let identity = FileIdentity::from_file(
            directory
                .try_clone()
                .context("Failed to clone bound development root capability")?
                .into_std_file(),
        )
        .context("Failed to inspect bound development root identity")?;
        let canonical_identity = FileIdentity::from_path(&canonical_root).map_err(|error| anyhow!(
            "workspace.path.not-confined: development root changed after binding `{}` while verifying `{}`: {error}",granted_root.display(),canonical_root.display()))?;
        if identity != canonical_identity {
            bail!(
                "workspace.path.not-confined: development root changed after binding `{}`; canonical path now identifies `{}`",
                granted_root.display(),
                canonical_root.display()
            );
        }
        Ok(Self {
            directory,
            granted_root,
            canonical_root,
            identity,
        })
    }
}

/// Immutable configuration input captured by a [`WorkspaceDiscoveryRoot`].
///
/// Decoding consumes only these owned values, even if files or environment
/// variables later change. This is a configuration snapshot, not a filesystem
/// transaction or evidence that the workspace remains current. Debug output
/// omits captured configuration and environment payloads.
pub struct CapturedWorkspaceDiscovery {
    canonical_root: PathBuf,
    request: morphir_workspace::DiscoveryRequest,
}
impl std::fmt::Debug for CapturedWorkspaceDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapturedWorkspaceDiscovery")
            .field("canonical_root", &self.canonical_root)
            .finish_non_exhaustive()
    }
}
impl CapturedWorkspaceDiscovery {
    pub(super) fn into_parts(self) -> (PathBuf, morphir_workspace::DiscoveryRequest) {
        (self.canonical_root, self.request)
    }

    /// Run portable discovery and decode its effective configurations without I/O.
    ///
    /// The returned native snapshot describes portable discovery over this input.
    /// An installed-provider host must keep the actual guest response as its
    /// membership authority; these local typed details can supply configuration
    /// sidecars but must not replace the guest's project inventory.
    pub fn into_typed_details(
        self,
    ) -> Result<super::NativeWorkspaceDiscovery, super::NativeWorkspaceDiscoveryError> {
        super::decode_captured_details(self.canonical_root, self.request)
    }
    /// Borrow the exact request to send to an installed discovery guest.
    ///
    /// Serialize or clone this request before consuming the capture for typed
    /// details. Do not recapture the directory for that sidecar, since another
    /// capture may observe different files, mounts or environment values.
    pub fn request(&self) -> &morphir_workspace::DiscoveryRequest {
        &self.request
    }
}
impl WorkspaceDiscoveryRoot {
    /// Capture one request using the existing confined traversal and source policy.
    ///
    /// Development reads use the supplied directory capability. System/global
    /// sources, user overrides and environment variables remain explicit grants
    /// selected by `options`; defaults retain their historical process behavior.
    /// Use [`super::ConfigLoadOptions::project_only`] to omit those other sources.
    ///
    /// Existing budgets cover 65,536 native directories, 262,144 native entries,
    /// depth 128 and 64 MiB of retained configuration text, including alias copies
    /// and selected mounts. Independent alias work limits also remain in force.
    /// This synchronous operation does not reserve host worker capacity, provide
    /// cancellation or freeze concurrent filesystem changes. The caller owns
    /// those lifecycle policies and any later currentness check.
    pub fn capture(
        &self,
        options: &super::ConfigLoadOptions,
    ) -> anyhow::Result<CapturedWorkspaceDiscovery> {
        self.capture_with_budgets(
            options,
            super::AliasBudgets::DEFAULT,
            super::TraversalBudgets::DEFAULT,
        )
    }
    pub(super) fn capture_with_budgets(
        &self,
        options: &super::ConfigLoadOptions,
        aliases: super::AliasBudgets,
        traversal: super::TraversalBudgets,
    ) -> anyhow::Result<CapturedWorkspaceDiscovery> {
        self.verify_current()?;
        let request = super::capture_request(
            &self.directory,
            &self.canonical_root,
            &self.granted_root,
            options,
            aliases,
            traversal,
        )?;
        Ok(CapturedWorkspaceDiscovery {
            canonical_root: self.canonical_root.clone(),
            request,
        })
    }
    fn verify_current(&self) -> anyhow::Result<()> {
        for label in [&self.granted_root, &self.canonical_root] {
            let identity = FileIdentity::from_path(label).map_err(|error| anyhow!(
                "workspace.path.not-confined: development root changed after binding `{}` while verifying `{}`: {error}", self.granted_root.display(), label.display()))?;
            if identity != self.identity {
                bail!(
                    "workspace.path.not-confined: development root changed after binding `{}`; label `{}` no longer identifies the owned directory",
                    self.granted_root.display(),
                    label.display()
                );
            }
        }
        Ok(())
    }
}

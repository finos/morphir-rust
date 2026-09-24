//! An installed extension as a registry source.
//!
//! Unlike [`crate::NativeSource`], which wraps an extension already loaded
//! into the host process, an [`InstalledSource`] holds only the atomically
//! validated catalog and lock pair the daemon reads through
//! `morphir_distribution::list_installed`. Its guest is verified and started
//! only when [`InstalledSource::connect`] runs.

use crate::activate;
use async_trait::async_trait;
use morphir_common::home::MorphirHome;
use morphir_distribution::{
    ArtifactRuntime, InstalledExtensionSnapshot, activate_installed_snapshot,
};
use morphir_extension_sdk::{ExtensionCapabilities, ExtensionInfo};
use morphir_host::{
    CapabilityMetadataScope, GuestConnection, GuestSource, HostError, InvocationMode,
    InvocationPolicy, ProviderOrigin,
};
use std::path::Path;

/// An extension selected from the installed catalog.
///
/// Its origin is `Installed` and its capability metadata is the persisted
/// frontend and backend members the installed snapshot recorded. Its
/// invocation mode follows the installed artifact's runtime -- `ProcessMep`
/// for a process artifact, `WasmMep` for a WebAssembly artifact -- under
/// every [`InvocationPolicy`].
pub struct InstalledSource {
    home: MorphirHome,
    snapshot: InstalledExtensionSnapshot,
    info: ExtensionInfo,
    capabilities: ExtensionCapabilities,
    mode: InvocationMode,
}

impl InstalledSource {
    /// Offer `snapshot`, activated below `home`, to a registry.
    pub fn new(home: MorphirHome, snapshot: InstalledExtensionSnapshot) -> Self {
        let info = snapshot.installed().extension_info();
        let capabilities = snapshot.installed().extension_capabilities();
        let mode = match snapshot.installed().runtime() {
            ArtifactRuntime::Process => InvocationMode::ProcessMep,
            ArtifactRuntime::Wasm => InvocationMode::WasmMep,
        };
        Self {
            home,
            snapshot,
            info,
            capabilities,
            mode,
        }
    }

    /// The atomically validated installed snapshot this source registered from.
    pub fn snapshot(&self) -> &InstalledExtensionSnapshot {
        &self.snapshot
    }
}

#[async_trait]
impl GuestSource for InstalledSource {
    fn info(&self) -> &ExtensionInfo {
        &self.info
    }

    fn capabilities(&self) -> &ExtensionCapabilities {
        &self.capabilities
    }

    fn origin(&self) -> ProviderOrigin {
        ProviderOrigin::Installed
    }

    fn capability_metadata_scope(&self) -> CapabilityMetadataScope {
        CapabilityMetadataScope::PersistedFrontendBackend
    }

    fn invocation_mode(&self, _policy: InvocationPolicy) -> InvocationMode {
        self.mode
    }

    async fn connect(&self, workspace: &Path) -> Result<Box<dyn GuestConnection>, HostError> {
        let id = &self.info.id;
        let artifact =
            activate_installed_snapshot(&self.home, &self.snapshot).map_err(|error| {
                HostError::Invalid(format!(
                    "Failed to verify installed provider '{id}': {error}"
                ))
            })?;
        let guest = activate(artifact, workspace).await.map_err(|error| {
            HostError::Invalid(format!(
                "Failed to activate installed provider '{id}': {error}"
            ))
        })?;
        Ok(Box::new(guest.connection))
    }
}

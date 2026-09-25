//! An installed extension as a registry source.
//!
//! Unlike [`crate::NativeSource`], which wraps an extension already loaded
//! into the host process, an [`InstalledSource`] holds only the atomically
//! validated catalog and lock pair the daemon reads through
//! `morphir_distribution::list_installed`. Its guest is verified and started
//! only when [`InstalledSource::activate`] (or its [`GuestSource::connect`])
//! runs.

use crate::{ActivatedGuest, activate};
use async_trait::async_trait;
use morphir_common::home::MorphirHome;
use morphir_distribution::{
    ArtifactRuntime, DistributionError, InstalledExtensionSnapshot, activate_installed_snapshot,
};
use morphir_extension_sdk::{ExtensionCapabilities, ExtensionInfo};
use morphir_host::{
    CapabilityMetadataScope, GuestConnection, GuestSource, HostError, InvocationMode,
    InvocationPolicy, ProviderOrigin,
};
use std::path::Path;

/// Why [`InstalledSource::activate`] could not start an installed guest.
///
/// The `Display` texts are the texts [`GuestSource::connect`] reports. A
/// caller that words its own texts matches on the variant and reads the
/// inner error instead.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum InstalledSourceError {
    /// The installed artifact failed verification against its lock.
    #[error("Failed to verify installed provider '{id}': {error}")]
    Verify {
        /// The installed extension id.
        id: String,
        /// Why verification failed.
        #[source]
        error: DistributionError,
    },
    /// The verification worker stopped before it returned: it panicked, or
    /// the runtime shut down.
    #[error("Failed to verify installed provider '{id}': verification worker failed: {message}")]
    VerifyWorker {
        /// The installed extension id.
        id: String,
        /// What the worker reported.
        message: String,
    },
    /// The verified artifact did not start.
    #[error("Failed to activate installed provider '{id}': {error}")]
    Activate {
        /// The installed extension id.
        id: String,
        /// Why the guest did not start. The variant, channel state and cause
        /// are the ones [`activate`] reported. Boxed to keep the error small.
        #[source]
        error: Box<HostError>,
    },
}

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

    /// Verify the installed artifact and start its guest in `workspace`,
    /// without the MEP handshake.
    ///
    /// Verification reads and hashes the installed bytes, so it runs on a
    /// blocking worker. Unlike [`GuestSource::connect`], which flattens
    /// every failure into [`HostError::Invalid`], this keeps the failure
    /// typed.
    pub async fn activate(&self, workspace: &Path) -> Result<ActivatedGuest, InstalledSourceError> {
        let id = self.info.id.clone();
        let home = self.home.clone();
        let snapshot = self.snapshot.clone();
        let artifact =
            tokio::task::spawn_blocking(move || activate_installed_snapshot(&home, &snapshot))
                .await
                .map_err(|error| InstalledSourceError::VerifyWorker {
                    id: id.clone(),
                    message: error.to_string(),
                })?
                .map_err(|error| InstalledSourceError::Verify {
                    id: id.clone(),
                    error,
                })?;
        activate(artifact, workspace)
            .await
            .map_err(|error| InstalledSourceError::Activate {
                id,
                error: Box::new(error),
            })
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

    /// [`InstalledSource::activate`], with its failure flattened into
    /// [`HostError::Invalid`] carrying the same text.
    async fn connect(&self, workspace: &Path) -> Result<Box<dyn GuestConnection>, HostError> {
        let guest = self
            .activate(workspace)
            .await
            .map_err(|error| HostError::Invalid(error.to_string()))?;
        Ok(Box::new(guest.connection))
    }
}

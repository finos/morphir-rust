//! Verified acquisition and selection of Morphir extension artifacts.
//!
//! The crate separates pure index resolution from filesystem acquisition. A
//! caller can therefore inspect and resolve controlled metadata before any
//! bytes are copied into the Morphir artifact store.
//!
//! ```no_run
//! use morphir_common::home::MorphirHome;
//! use morphir_distribution::{
//!     Channel, ExtensionId, ExtensionInstaller, LocalIndex, Platform, Selection,
//!     VerifiedExtensionArtifact, activate_installed, list_installed, uninstall_extension,
//! };
//!
//! # fn install() -> Result<(), Box<dyn std::error::Error>> {
//! let home = MorphirHome::resolve()?;
//! let id = ExtensionId::parse("morphir-elm")?;
//! let selected = LocalIndex::open("./controlled-index")?.resolve(
//!     &id,
//!     Selection::Channel(Channel::Stable),
//!     &Platform::current(),
//! )?;
//! ExtensionInstaller::new(&home).install(selected)?;
//!
//! // Activation is offline and rehashes the installed bytes.
//! let artifact = activate_installed(&home, &id)?;
//! match artifact {
//!     VerifiedExtensionArtifact::Process(process) => {
//!         assert_eq!(process.extension_info().id, "morphir-elm");
//!     }
//!     VerifiedExtensionArtifact::Wasm(_) => unreachable!("selected a process artifact"),
//! }
//!
//! // Catalog entries and their exact locks are read as one validated snapshot.
//! let installed = list_installed(&home)?;
//! assert_eq!(installed[0].installed().extension_id(), &id);
//! assert_eq!(
//!     installed[0].selection(),
//!     &Selection::Channel(Channel::Stable),
//! );
//!
//! // Uninstall removes active state but leaves content-addressed bytes cached.
//! let removed = uninstall_extension(&home, &id)?;
//! assert_eq!(removed.extension_id(), &id);
//! # Ok(())
//! # }
//! ```

mod domain;
mod error;
mod index;
mod local;
mod resolver;
mod state;
mod store;

pub use domain::{
    ArtifactFilename, ArtifactRecord, ArtifactRuntime, ArtifactSource, BackendRecord, Capability,
    Channel, ChannelSegment, ExtensionId, Platform, RelativeArtifactPath, ReleaseRecord, Selection,
    Sha256Digest,
};
pub use error::{DistributionError, Result};
pub use index::ExtensionHistory;
pub use local::{IndexKind, IndexProvenance, LocalIndex, ResolvedArtifact};
pub use resolver::{ResolvedRelease, resolve};
pub use state::{
    ExtensionInstaller, ExtensionLock, InstalledCatalog, InstalledExtension,
    InstalledExtensionSnapshot, VerifiedExtensionArtifact, VerifiedProcessArtifact,
    VerifiedWasmArtifact, activate_installed, activate_installed_snapshot, list_installed,
    read_extension_lock, uninstall_extension, write_extension_lock,
};
pub use store::{ArtifactStore, VerifiedArtifact};

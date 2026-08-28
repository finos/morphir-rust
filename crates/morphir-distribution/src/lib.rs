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
//!     activate_installed,
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
//! let process = activate_installed(&home, &id)?;
//! assert_eq!(process.extension_info().id, "morphir-elm");
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
    ArtifactFilename, ArtifactRecord, ArtifactRuntime, ArtifactSource, Capability, Channel,
    ChannelSegment, ExtensionId, Platform, RelativeArtifactPath, ReleaseRecord, Selection,
    Sha256Digest,
};
pub use error::{DistributionError, Result};
pub use index::ExtensionHistory;
pub use local::{IndexKind, IndexProvenance, LocalIndex, ResolvedArtifact};
pub use resolver::{ResolvedRelease, resolve};
pub use state::{
    ExtensionInstaller, ExtensionLock, InstalledCatalog, InstalledExtension,
    VerifiedProcessArtifact, activate_installed, read_extension_lock, write_extension_lock,
};
pub use store::{ArtifactStore, VerifiedArtifact};

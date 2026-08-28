//! Verified acquisition and selection of Morphir extension artifacts.
//!
//! The crate separates pure index resolution from filesystem acquisition. A
//! caller can therefore inspect and resolve controlled metadata before any
//! bytes are copied into the Morphir artifact store.

mod domain;
mod error;
mod index;
mod resolver;

pub use domain::{
    ArtifactFilename, ArtifactRecord, ArtifactRuntime, ArtifactSource, Capability, Channel,
    ChannelSegment, ExtensionId, Platform, RelativeArtifactPath, ReleaseRecord, Selection,
    Sha256Digest,
};
pub use error::{DistributionError, Result};
pub use index::ExtensionHistory;
pub use resolver::{ResolvedRelease, resolve};

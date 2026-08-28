//! Immutable values used by extension indexes and locks.

mod digest;
mod identity;
mod manifest;

pub use digest::Sha256Digest;
pub use identity::{ArtifactFilename, Channel, ChannelSegment, ExtensionId, RelativeArtifactPath};
pub use manifest::{
    ArtifactRecord, ArtifactRuntime, ArtifactSource, Capability, Platform, ReleaseRecord, Selection,
};

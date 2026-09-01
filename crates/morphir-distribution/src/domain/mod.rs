//! Immutable values used by extension indexes and locks.

mod digest;
mod identity;
mod manifest;
mod tool;

pub use digest::Sha256Digest;
pub(crate) use identity::portable_token;
pub use identity::{
    ArtifactFilename, Channel, ChannelSegment, ExtensionId, RelativeArtifactPath, ToolId,
};
pub use manifest::{
    ArtifactRecord, ArtifactRuntime, ArtifactSource, BackendRecord, Capability,
    FrontendLanguageRecord, FrontendRecord, Platform, ReleaseRecord, Selection,
};
pub use tool::{
    ArchiveFormat, ToolArchive, ToolArtifactRecord, ToolLaunch, ToolReleaseRecord,
    ToolReleaseStatus,
};

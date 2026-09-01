//! Immutable values used by extension indexes and locks.

mod digest;
mod identity;
mod manifest;
mod schema_version;
mod tool;

pub use digest::Sha256Digest;
pub(crate) use identity::portable_token;
pub use identity::{
    ArtifactFilename, Channel, ChannelSegment, ExtensionId, RelativeArtifactPath, ToolId,
};
pub(crate) use manifest::supports_release_schema_version;
pub use manifest::{
    ArtifactRecord, ArtifactRuntime, ArtifactSource, BackendRecord, CURRENT_RELEASE_SCHEMA_VERSION,
    Capability, FrontendLanguageRecord, FrontendRecord, MINIMUM_RELEASE_SCHEMA_VERSION, Platform,
    ReleaseRecord, Selection,
};
pub use schema_version::SchemaVersion;
pub use tool::{
    ArchiveFormat, ToolArchive, ToolArtifactRecord, ToolLaunch, ToolReleaseRecord,
    ToolReleaseStatus,
};

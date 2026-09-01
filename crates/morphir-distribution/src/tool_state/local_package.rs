//! Explicit unsigned local-development package input.

use crate::{ArchiveFormat, DistributionError, Platform, RelativeArtifactPath, Result, ToolId};
use semver::Version;
use std::path::PathBuf;

/// An unsigned package accepted only through the explicit local developer policy.
#[derive(Debug)]
pub struct LocalDeveloperToolPackage {
    pub(super) source: PathBuf,
    pub(super) tool_id: ToolId,
    pub(super) tool_name: String,
    pub(super) version: Version,
    pub(super) platform: Platform,
    pub(super) format: ArchiveFormat,
    pub(super) entry_point: RelativeArtifactPath,
    pub(super) args: Vec<String>,
}

impl LocalDeveloperToolPackage {
    /// Describe one local package. Integrity metadata is computed from the source at preparation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: PathBuf,
        tool_id: ToolId,
        tool_name: impl Into<String>,
        version: Version,
        platform: Platform,
        format: ArchiveFormat,
        entry_point: RelativeArtifactPath,
        args: Vec<String>,
    ) -> Result<Self> {
        let tool_name = tool_name.into();
        if tool_name.trim().is_empty() {
            return Err(DistributionError::InvalidToolManifest {
                reason: "local developer tool name cannot be blank".to_owned(),
            });
        }
        entry_point.validate_declared()?;
        if args.iter().any(|argument| argument.contains('\0')) {
            return Err(DistributionError::InvalidToolManifest {
                reason: "local developer launch arguments cannot contain NUL".to_owned(),
            });
        }
        Ok(Self {
            source,
            tool_id,
            tool_name,
            version,
            platform,
            format,
            entry_point,
            args,
        })
    }
}

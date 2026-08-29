//! Pure resolution of authenticated tool release descriptors.

use crate::{
    Channel, DistributionError, Platform, Result, Selection, ToolArtifactRecord, ToolReleaseRecord,
    ToolReleaseStatus,
};
use semver::Version;

/// An exact tool release and its single platform artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedToolRelease {
    release: ToolReleaseRecord,
    artifact: ToolArtifactRecord,
    selection: Selection,
}

impl ResolvedToolRelease {
    /// Return the exact selected tool release.
    pub fn release(&self) -> &ToolReleaseRecord {
        &self.release
    }

    /// Return the selected platform artifact.
    pub fn artifact(&self) -> &ToolArtifactRecord {
        &self.artifact
    }

    /// Return the channel or exact-version request.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }
}

/// Select the highest compatible tool release and one platform artifact.
pub fn resolve_tool(
    releases: &[ToolReleaseRecord],
    selection: &Selection,
    platform: &Platform,
    morphir_cli: &Version,
) -> Result<ResolvedToolRelease> {
    let matching = releases
        .iter()
        .filter(|release| matches_selection(release, selection))
        .collect::<Vec<_>>();

    if let Selection::Exact(version) = selection
        && let Some(release) = matching
            .iter()
            .find(|release| release.status() == ToolReleaseStatus::Revoked)
    {
        return Err(DistributionError::RevokedToolRelease {
            tool: release.tool_id().clone(),
            version: version.clone(),
        });
    }

    let selectable = matching
        .iter()
        .copied()
        .filter(|release| release.status() != ToolReleaseStatus::Revoked)
        .collect::<Vec<_>>();
    let mut compatible = selectable
        .iter()
        .copied()
        .filter(|release| release.morphir_cli_requirement().matches(morphir_cli))
        .collect::<Vec<_>>();
    if !selectable.is_empty() && compatible.is_empty() {
        return Err(DistributionError::NoCompatibleCliVersion {
            selection: selection.to_string(),
            cli_version: morphir_cli.clone(),
        });
    }
    compatible.sort_by(|left, right| right.version().cmp_precedence(left.version()));

    for release in compatible {
        let artifacts = release
            .artifacts()
            .iter()
            .filter(|artifact| artifact.platform() == platform)
            .collect::<Vec<_>>();
        match artifacts.as_slice() {
            [] => continue,
            [artifact] => {
                return Ok(ResolvedToolRelease {
                    release: release.clone(),
                    artifact: (*artifact).clone(),
                    selection: selection.clone(),
                });
            }
            _ => {
                return Err(DistributionError::AmbiguousPlatform {
                    version: release.version().clone(),
                    platform: platform.to_string(),
                });
            }
        }
    }

    Err(DistributionError::NoMatchingArtifact {
        selection: selection.to_string(),
        platform: platform.to_string(),
    })
}

fn matches_selection(release: &ToolReleaseRecord, selection: &Selection) -> bool {
    match selection {
        Selection::Exact(version) => release.version() == version,
        Selection::Channel(Channel::Stable) => {
            release.status() == ToolReleaseStatus::Active
                && release.version().pre.is_empty()
                && release.channels().contains(&Channel::Stable)
        }
        Selection::Channel(Channel::Preview(None) | Channel::Insiders) => {
            release.status() == ToolReleaseStatus::Active
                && release
                    .channels()
                    .iter()
                    .any(|channel| matches!(channel, Channel::Preview(_) | Channel::Insiders))
        }
        Selection::Channel(Channel::Preview(Some(expected))) => release.status()
            == ToolReleaseStatus::Active
            && release.channels().iter().any(
                |channel| matches!(channel, Channel::Preview(Some(actual)) if actual == expected),
            ),
    }
}

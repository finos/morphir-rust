//! Pure platform and release selection.

use crate::DistributionError;
use crate::{
    ArtifactRecord, Channel, ExtensionHistory, Platform, ReleaseRecord, Result, Selection,
};

/// An exact release and its single platform artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRelease {
    release: ReleaseRecord,
    artifact: ArtifactRecord,
    selection: Selection,
}

impl ResolvedRelease {
    /// Return the exact selected release record.
    pub fn release(&self) -> &ReleaseRecord {
        &self.release
    }

    /// Return the selected platform artifact.
    pub fn artifact(&self) -> &ArtifactRecord {
        &self.artifact
    }

    /// Return the original request, including the `insiders` spelling.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }
}

/// Select the highest compatible exact release and one platform artifact.
pub fn resolve(
    history: &ExtensionHistory,
    selection: &Selection,
    platform: &Platform,
) -> Result<ResolvedRelease> {
    let mut candidates = history
        .releases()
        .iter()
        .filter(|release| matches_selection(release, selection))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.version().cmp_precedence(left.version()));

    for release in candidates {
        let artifacts = release
            .artifacts()
            .iter()
            .filter(|artifact| artifact.platform() == platform)
            .collect::<Vec<_>>();
        match artifacts.as_slice() {
            [] => continue,
            [artifact] => {
                return Ok(ResolvedRelease {
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

fn matches_selection(release: &ReleaseRecord, selection: &Selection) -> bool {
    match selection {
        Selection::Exact(version) => release.version() == version,
        Selection::Channel(Channel::Stable) => {
            release.version().pre.is_empty() && release.channels().contains(&Channel::Stable)
        }
        Selection::Channel(Channel::Preview(None) | Channel::Insiders) => release
            .channels()
            .iter()
            .any(|channel| matches!(channel, Channel::Preview(_) | Channel::Insiders)),
        Selection::Channel(Channel::Preview(Some(expected))) => release
            .channels()
            .iter()
            .any(|channel| matches!(channel, Channel::Preview(Some(actual)) if actual == expected)),
    }
}

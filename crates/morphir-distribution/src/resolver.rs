//! Pure platform and release selection.

use crate::DistributionError;
use crate::{
    ArtifactRecord, ArtifactRuntime, Channel, ExtensionHistory, Platform, ReleaseRecord, Result,
    Selection,
};
use morphir_extension_sdk::protocol::SUPPORTED_MEP_VERSIONS;

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
    let matching_selection = history
        .releases()
        .iter()
        .filter(|release| matches_selection(release, selection))
        .collect::<Vec<_>>();
    let mut candidates = matching_selection
        .iter()
        .copied()
        .filter(|release| supports_host_mep(release))
        .collect::<Vec<_>>();
    if !matching_selection.is_empty() && candidates.is_empty() {
        return Err(DistributionError::NoCompatibleMepVersion {
            selection: selection.to_string(),
            supported: SUPPORTED_MEP_VERSIONS.join(", "),
        });
    }
    candidates.sort_by(|left, right| right.version().cmp_precedence(left.version()));

    for release in candidates {
        let artifacts = release
            .artifacts()
            .iter()
            .filter(|artifact| match artifact.runtime() {
                ArtifactRuntime::Process => artifact.platform() == Some(platform),
                ArtifactRuntime::Wasm => true,
            })
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

fn supports_host_mep(release: &ReleaseRecord) -> bool {
    release
        .mep_versions()
        .iter()
        .any(|version| SUPPORTED_MEP_VERSIONS.contains(&version.as_str()))
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

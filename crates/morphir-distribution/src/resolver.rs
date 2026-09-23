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
    /// Check both release and selected artifact requirements for this host.
    pub fn check_host(&self, host: &semver::Version) -> Result<()> {
        self.release.check_host(host)?;
        self.artifact.check_host(host)
    }

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
/// Refuse if its requirements are not met by the caller's host version.
pub fn resolve(
    history: &ExtensionHistory,
    selection: &Selection,
    platform: &Platform,
    host: &semver::Version,
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

    // A release whose statement needs a newer host is skipped, so a moving
    // channel still resolves the newest release this host can run. The host
    // error is reported only when no candidate fits.
    let mut host_refusal = None;
    for release in candidates {
        let artifacts = release
            .artifacts()
            .iter()
            .filter(|artifact| match artifact.runtime() {
                ArtifactRuntime::Process => artifact.platform() == Some(platform),
                ArtifactRuntime::Wasm => true,
            })
            .filter(|artifact| supports_artifact_mep(artifact))
            .collect::<Vec<_>>();
        match artifacts.as_slice() {
            [] => continue,
            [artifact] => {
                if let Err(error) = release
                    .check_host(host)
                    .and_then(|()| artifact.check_host(host))
                {
                    host_refusal.get_or_insert(error);
                    continue;
                }
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

    if let Some(error) = host_refusal {
        return Err(error);
    }
    Err(DistributionError::NoMatchingArtifact {
        selection: selection.to_string(),
        platform: platform.to_string(),
    })
}

fn supports_host_mep(release: &ReleaseRecord) -> bool {
    release.artifacts().iter().any(supports_artifact_mep)
}

fn supports_artifact_mep(artifact: &ArtifactRecord) -> bool {
    artifact.statement().is_some_and(|statement| {
        statement
            .protocol_versions
            .iter()
            .any(|version| SUPPORTED_MEP_VERSIONS.contains(&version.as_str()))
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

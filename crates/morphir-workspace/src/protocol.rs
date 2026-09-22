//! The workspace discovery protocol version.
//!
//! Versions follow the default contract versioning scheme: a SemVer string,
//! compatible within a released major (within the minor on `0.y.z`), with a
//! prerelease that matches only exactly. The protocol is still a draft, so it
//! is refined in place and a reader names the exact draft it speaks.

use semver::{Version, VersionReq};

/// The workspace discovery protocol version this build writes.
pub const WORKSPACE_DISCOVERY_PROTOCOL: &str = "0.1.0-draft.1";

/// The versions this build reads, as requirements: a caret for a released
/// line, an exact requirement for a draft. A caret alone would also admit
/// later drafts of the same release.
const SPOKEN: &[&str] = &["=0.1.0-draft.1"];

/// [`WORKSPACE_DISCOVERY_PROTOCOL`] as a version.
#[must_use]
pub fn workspace_discovery_protocol() -> Version {
    Version::parse(WORKSPACE_DISCOVERY_PROTOCOL)
        .expect("the workspace discovery protocol constant is a SemVer version")
}

/// Whether this build speaks workspace discovery protocol `version`.
#[must_use]
pub fn speaks_workspace_discovery_protocol(version: &Version) -> bool {
    SPOKEN.iter().any(|requirement| {
        VersionReq::parse(requirement)
            .expect("the spoken workspace discovery protocols are SemVer requirements")
            .matches(version)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_build_speaks_the_version_it_writes() {
        assert!(speaks_workspace_discovery_protocol(
            &workspace_discovery_protocol()
        ));
    }

    /// A draft matches only exactly: a later draft of the same release may
    /// have changed what this one meant, and the release is not the draft.
    #[test]
    fn a_draft_matches_only_exactly() {
        for other in ["0.1.0-draft.2", "0.1.0", "0.1.1", "0.2.0-draft.1", "1.0.0"] {
            assert!(
                !speaks_workspace_discovery_protocol(&Version::parse(other).unwrap()),
                "{other} must not be taken for {WORKSPACE_DISCOVERY_PROTOCOL}"
            );
        }
    }
}

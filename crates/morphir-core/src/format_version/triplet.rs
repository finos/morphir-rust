//! Exact three-component release triplets.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Largest value a component may take (the unsigned 32-bit range).
pub const COMPONENT_MAX: u32 = u32::MAX;

/// Exact normalized `N.minor.patch` release.
///
/// The fields are declared `major`, `minor`, `patch`, so the derived order is
/// the lexicographic release order the contract uses.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub struct ReleaseTriplet {
    major: u32,
    minor: u32,
    patch: u32,
}

impl ReleaseTriplet {
    /// Create a release triplet from its three components.
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Return the major component.
    pub const fn major(self) -> u32 {
        self.major
    }

    /// Return the minor component.
    pub const fn minor(self) -> u32 {
        self.minor
    }

    /// Return the patch component.
    pub const fn patch(self) -> u32 {
        self.patch
    }

    /// Return `true` when this is the baseline release `N.0.0`.
    pub const fn is_baseline(self) -> bool {
        self.minor == 0 && self.patch == 0
    }

    /// Render the exact release as `N.minor.patch`.
    pub fn to_exact_string(self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }

    /// The next release for canonicalisation, or `None` when the patch is at
    /// its maximum.
    ///
    /// Deliberately non-carrying, so an inclusive bound at the patch maximum
    /// keeps its bracket rather than moving to a release of another minor the
    /// author did not write.
    pub fn next(&self) -> Option<Self> {
        (self.patch != COMPONENT_MAX).then(|| Self::new(self.major, self.minor, self.patch + 1))
    }

    /// The release immediately after this one in release order.
    ///
    /// The next patch, carrying into the next minor and then the next major,
    /// and `None` for the maximum release, which has no successor. This is the
    /// ordering question — which release an exclusive bound actually admits
    /// first — and not the canonical spelling question [`Self::next`] answers.
    pub fn successor(&self) -> Option<Self> {
        if self.patch != COMPONENT_MAX {
            return Some(Self::new(self.major, self.minor, self.patch + 1));
        }
        if self.minor != COMPONENT_MAX {
            return Some(Self::new(self.major, self.minor + 1, 0));
        }
        if self.major != COMPONENT_MAX {
            return Some(Self::new(self.major + 1, 0, 0));
        }
        None
    }
}

impl std::fmt::Display for ReleaseTriplet {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

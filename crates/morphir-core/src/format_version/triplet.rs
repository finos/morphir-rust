//! Exact three-component release triplets.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Exact normalized `N.minor.patch` release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
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
}

impl std::fmt::Display for ReleaseTriplet {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

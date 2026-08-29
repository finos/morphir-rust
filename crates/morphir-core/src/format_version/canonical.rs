//! Canonical wire spellings for normalized releases.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::triplet::ReleaseTriplet;

/// Canonical wire representation of one normalized release.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum CanonicalSpelling {
    /// Baseline release `N.0.0` written as integer `N`.
    Integer(u32),
    /// Non-baseline release written as exact string `N.minor.patch`.
    String(String),
}

impl CanonicalSpelling {
    /// Return the major family encoded by this spelling.
    pub fn major(&self) -> u32 {
        match self {
            Self::Integer(major) => *major,
            Self::String(value) => value
                .split('.')
                .next()
                .and_then(|part| part.parse().ok())
                .unwrap_or(0),
        }
    }
}

/// Compute the canonical wire spelling for one normalized release.
pub fn canonical_spelling(release: &ReleaseTriplet) -> CanonicalSpelling {
    if release.is_baseline() {
        CanonicalSpelling::Integer(release.major())
    } else {
        CanonicalSpelling::String(release.to_exact_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_releases_use_integer_spelling() {
        assert_eq!(
            canonical_spelling(&ReleaseTriplet::new(3, 0, 0)),
            CanonicalSpelling::Integer(3)
        );
    }

    #[test]
    fn nonbaseline_releases_use_exact_strings() {
        assert_eq!(
            canonical_spelling(&ReleaseTriplet::new(3, 1, 0)),
            CanonicalSpelling::String("3.1.0".into())
        );
    }
}

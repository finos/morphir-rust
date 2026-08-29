//! Support-table compatibility checks.

use super::diagnostic::FormatVersionDiagnostic;
use super::triplet::ReleaseTriplet;

/// Compatibility result after normalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compatibility {
    /// The exact normalized release is supported.
    Supported,
    /// The major family is recognized but no supported release exists.
    UnsupportedMajor,
    /// The major family is supported but not this exact release.
    UnsupportedRevision,
}

/// Explicit table of exact supported normalized releases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportTable {
    releases: Vec<ReleaseTriplet>,
}

impl SupportTable {
    /// Create a support table from an arbitrary list of exact releases.
    pub fn from_releases(releases: impl IntoIterator<Item = ReleaseTriplet>) -> Self {
        Self {
            releases: releases.into_iter().collect(),
        }
    }

    /// Return the reference support table from the parent specification.
    pub fn reference() -> Self {
        Self::from_releases([ReleaseTriplet::new(3, 0, 0), ReleaseTriplet::new(4, 0, 0)])
    }

    /// Check compatibility for one normalized release.
    pub fn check(&self, release: &ReleaseTriplet) -> Compatibility {
        if self.releases.iter().any(|supported| supported == release) {
            return Compatibility::Supported;
        }
        let major_supported = self
            .releases
            .iter()
            .any(|supported| supported.major() == release.major());
        if major_supported {
            Compatibility::UnsupportedRevision
        } else {
            Compatibility::UnsupportedMajor
        }
    }

    /// Convert a compatibility result into a stable diagnostic when unsupported.
    pub fn unsupported_diagnostic(
        &self,
        release: &ReleaseTriplet,
        compatibility: Compatibility,
    ) -> Option<FormatVersionDiagnostic> {
        match compatibility {
            Compatibility::Supported => None,
            Compatibility::UnsupportedMajor => {
                Some(FormatVersionDiagnostic::unsupported_format_version_major(
                    &release.to_exact_string(),
                ))
            }
            Compatibility::UnsupportedRevision => Some(
                FormatVersionDiagnostic::unsupported_format_version_revision(
                    &release.to_exact_string(),
                ),
            ),
        }
    }
}

/// Reference support table from the parent specification.
pub fn default_support_table() -> SupportTable {
    SupportTable::reference()
}

/// Alias for the reference support table accessor.
#[allow(dead_code)]
pub const DEFAULT_SUPPORT_TABLE: fn() -> SupportTable = SupportTable::reference;

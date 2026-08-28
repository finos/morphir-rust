//! Controlled local-directory extension indexes.

use crate::{
    ArtifactRecord, DistributionError, ExtensionHistory, ExtensionId, Platform, ReleaseRecord,
    Result, Selection, Sha256Digest, resolve,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Kind of controlled index used to select an artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IndexKind {
    /// A directory on the local filesystem.
    LocalDirectory,
}

impl IndexKind {
    /// Return the stable serialized index-kind spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalDirectory => "local-directory",
        }
    }
}

/// Exact index metadata used to resolve an artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndexProvenance {
    kind: IndexKind,
    identity: PathBuf,
    revision: Sha256Digest,
}

impl IndexProvenance {
    /// Return the index backend kind.
    pub fn kind(&self) -> IndexKind {
        self.kind
    }

    /// Return the canonical local index root.
    pub fn identity(&self) -> &Path {
        &self.identity
    }

    /// Return the SHA-256 digest of the exact selected history bytes.
    pub fn revision(&self) -> &Sha256Digest {
        &self.revision
    }
}

/// A controlled local-directory extension index.
#[derive(Debug, Clone)]
pub struct LocalIndex {
    root: PathBuf,
}

impl LocalIndex {
    /// Open and canonicalize an existing local index directory.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let requested = root.as_ref();
        let root = fs::canonicalize(requested).map_err(|source| DistributionError::Io {
            path: requested.to_path_buf(),
            source,
        })?;
        if !root.is_dir() {
            return Err(DistributionError::Io {
                path: root,
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "local index root is not a directory",
                ),
            });
        }
        Ok(Self { root })
    }

    /// Return the canonical controlled index root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Read the requested history and resolve one exact platform artifact.
    pub fn resolve(
        &self,
        extension_id: &ExtensionId,
        selection: Selection,
        platform: &Platform,
    ) -> Result<ResolvedArtifact> {
        let history_path = self
            .root
            .join("extensions")
            .join(format!("{extension_id}.jsonl"));
        let canonical_history =
            fs::canonicalize(&history_path).map_err(|source| DistributionError::Io {
                path: history_path,
                source,
            })?;
        ensure_contained(&self.root, &canonical_history)?;
        let bytes = fs::read(&canonical_history).map_err(|source| DistributionError::Io {
            path: canonical_history.clone(),
            source,
        })?;
        let history = ExtensionHistory::parse_jsonl(&bytes)?;
        if history.extension_id() != extension_id {
            return Err(DistributionError::MixedIdentity {
                expected: extension_id.to_string(),
                actual: history.extension_id().to_string(),
                line: 1,
            });
        }
        let resolved = resolve(&history, &selection, platform)?;
        Ok(ResolvedArtifact {
            release: resolved.release().clone(),
            artifact: resolved.artifact().clone(),
            selection,
            index: IndexProvenance {
                kind: IndexKind::LocalDirectory,
                identity: self.root.clone(),
                revision: history.revision().clone(),
            },
        })
    }
}

/// Exact metadata selected from a controlled index before byte verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedArtifact {
    pub(crate) release: ReleaseRecord,
    pub(crate) artifact: ArtifactRecord,
    pub(crate) selection: Selection,
    pub(crate) index: IndexProvenance,
}

impl ResolvedArtifact {
    /// Return the exact selected release.
    pub fn release(&self) -> &ReleaseRecord {
        &self.release
    }

    /// Return the selected platform artifact declaration.
    pub fn artifact(&self) -> &ArtifactRecord {
        &self.artifact
    }

    /// Return the channel or exact-version request that selected this release.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Return the exact index identity and revision.
    pub fn index(&self) -> &IndexProvenance {
        &self.index
    }
}

pub(crate) fn ensure_contained(root: &Path, path: &Path) -> Result<()> {
    if path.starts_with(root) {
        Ok(())
    } else {
        Err(DistributionError::IndexPathEscape {
            path: path.to_path_buf(),
            root: root.to_path_buf(),
        })
    }
}

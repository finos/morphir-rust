//! Digest verification and content-addressed publication.

use crate::local::ensure_contained;
use crate::{
    ArtifactSource, DistributionError, RelativeArtifactPath, ResolvedArtifact, Result, Sha256Digest,
};
use morphir_common::home::MorphirHome;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Content-addressed artifact store rooted in a Morphir home directory.
#[derive(Debug, Clone)]
pub struct ArtifactStore {
    home_root: PathBuf,
    root: PathBuf,
}

impl ArtifactStore {
    /// Construct the extension artifact store for one Morphir home.
    pub fn from_home(home: &MorphirHome) -> Self {
        Self {
            home_root: home.root().to_path_buf(),
            root: home.extensions_store_dir(),
        }
    }

    /// Return the SHA-256 content-addressed store root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Verify selected local bytes and atomically publish them into the store.
    ///
    /// Existing objects are hashed before reuse. A mismatch never overwrites
    /// the object because it may be evidence of local tampering.
    pub fn materialize(&self, selected: ResolvedArtifact) -> Result<VerifiedArtifact> {
        let source_path = match selected.artifact.source() {
            ArtifactSource::LocalFile { path } => selected.index.identity().join(path.as_path()),
        };
        let canonical_source =
            fs::canonicalize(&source_path).map_err(|source| DistributionError::Io {
                path: source_path,
                source,
            })?;
        ensure_contained(selected.index.identity(), &canonical_source)?;
        if !canonical_source.is_file() {
            return Err(DistributionError::Io {
                path: canonical_source,
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "artifact source is not a regular file",
                ),
            });
        }

        fs::create_dir_all(&self.root).map_err(|source| DistributionError::Io {
            path: self.root.clone(),
            source,
        })?;
        let canonical_home = canonicalize(&self.home_root)?;
        let canonical_store = canonicalize(&self.root)?;
        ensure_home_contained(&canonical_home, &canonical_store)?;

        let requested_digest_directory = self.root.join(selected.artifact.digest().to_string());
        fs::create_dir_all(&requested_digest_directory).map_err(|source| {
            DistributionError::Io {
                path: requested_digest_directory.clone(),
                source,
            }
        })?;
        let digest_directory = canonicalize(&requested_digest_directory)?;
        ensure_home_contained(&canonical_store, &digest_directory).map_err(|_| {
            DistributionError::InstalledPathEscape {
                path: digest_directory.clone(),
                root: canonical_home.clone(),
            }
        })?;
        let destination = digest_directory.join(selected.artifact.filename().as_str());

        if destination.exists() {
            let canonical_destination = canonicalize(&destination)?;
            ensure_home_contained(&canonical_store, &canonical_destination).map_err(|_| {
                DistributionError::InstalledPathEscape {
                    path: canonical_destination.clone(),
                    root: canonical_home,
                }
            })?;
            verify_file(&canonical_destination, selected.artifact.digest())?;
            verify_executable_mode(&canonical_destination, selected.artifact.executable())?;
            return self.verified(selected, canonical_destination);
        }

        let mut staged = tempfile::Builder::new()
            .prefix(".stage-")
            .tempfile_in(&digest_directory)
            .map_err(|source| DistributionError::Io {
                path: digest_directory.clone(),
                source,
            })?;
        let actual = copy_and_hash(&canonical_source, staged.as_file_mut())?;
        if &actual != selected.artifact.digest() {
            return Err(DistributionError::DigestMismatch {
                path: canonical_source,
                expected: selected.artifact.digest().clone(),
                actual,
            });
        }
        staged
            .as_file_mut()
            .flush()
            .and_then(|()| staged.as_file().sync_all())
            .map_err(|source| DistributionError::Io {
                path: staged.path().to_path_buf(),
                source,
            })?;
        if selected.artifact.executable() {
            add_owner_executable(staged.path())?;
        }

        match staged.persist_noclobber(&destination) {
            Ok(_) => {
                verify_executable_mode(&destination, selected.artifact.executable())?;
                self.verified(selected, destination)
            }
            Err(_error) if destination.exists() => {
                let canonical_destination = canonicalize(&destination)?;
                ensure_home_contained(&canonical_store, &canonical_destination).map_err(|_| {
                    DistributionError::InstalledPathEscape {
                        path: canonical_destination.clone(),
                        root: canonical_home,
                    }
                })?;
                verify_file(&canonical_destination, selected.artifact.digest())?;
                verify_executable_mode(&canonical_destination, selected.artifact.executable())?;
                self.verified(selected, canonical_destination)
            }
            Err(error) => Err(DistributionError::Io {
                path: destination,
                source: error.error,
            }),
        }
    }

    fn verified(&self, selected: ResolvedArtifact, path: PathBuf) -> Result<VerifiedArtifact> {
        let canonical_home = canonicalize(&self.home_root)?;
        let relative = path
            .strip_prefix(&canonical_home)
            .map(Path::to_path_buf)
            .map_err(|_| DistributionError::InstalledPathEscape {
                path: path.clone(),
                root: canonical_home,
            })?;
        Ok(VerifiedArtifact {
            selected,
            path,
            store_path: RelativeArtifactPath::from_native_path(&relative)?,
        })
    }
}

fn canonicalize(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).map_err(|source| DistributionError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn ensure_home_contained(root: &Path, path: &Path) -> Result<()> {
    if path.starts_with(root) {
        Ok(())
    } else {
        Err(DistributionError::InstalledPathEscape {
            path: path.to_path_buf(),
            root: root.to_path_buf(),
        })
    }
}

/// Artifact bytes whose digest matched the selected controlled metadata.
///
/// The fields are private. The only public constructor is
/// [`ArtifactStore::materialize`], so catalog registration cannot accept raw
/// paths or unchecked bytes.
#[derive(Debug)]
pub struct VerifiedArtifact {
    pub(crate) selected: ResolvedArtifact,
    pub(crate) path: PathBuf,
    pub(crate) store_path: RelativeArtifactPath,
}

impl VerifiedArtifact {
    /// Return the verified materialized path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the verified SHA-256 digest.
    pub fn digest(&self) -> &Sha256Digest {
        self.selected.artifact.digest()
    }

    /// Return the exact selected metadata.
    pub fn selected(&self) -> &ResolvedArtifact {
        &self.selected
    }

    /// Return the materialized path relative to Morphir home.
    pub fn store_path(&self) -> &Path {
        self.store_path.as_path()
    }
}

pub(crate) fn verify_file(path: &Path, expected: &Sha256Digest) -> Result<()> {
    let actual = hash_file(path)?;
    if &actual == expected {
        Ok(())
    } else {
        Err(DistributionError::DigestMismatch {
            path: path.to_path_buf(),
            expected: expected.clone(),
            actual,
        })
    }
}

#[cfg(unix)]
pub(crate) fn verify_executable_mode(path: &Path, expected: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let actual = fs::metadata(path)
        .map_err(|source| DistributionError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .permissions()
        .mode()
        & 0o100
        != 0;
    if actual == expected {
        Ok(())
    } else {
        Err(DistributionError::ExecutableModeMismatch {
            path: path.to_path_buf(),
            expected,
            actual,
        })
    }
}

#[cfg(not(unix))]
pub(crate) fn verify_executable_mode(_path: &Path, _expected: bool) -> Result<()> {
    Ok(())
}

fn hash_file(path: &Path) -> Result<Sha256Digest> {
    let mut file = File::open(path).map_err(|source| DistributionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|source| DistributionError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(Sha256Digest::from_bytes(hasher.finalize().into()))
}

fn copy_and_hash(source: &Path, destination: &mut File) -> Result<Sha256Digest> {
    let mut source_file = File::open(source).map_err(|error| DistributionError::Io {
        path: source.to_path_buf(),
        source: error,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = source_file
            .read(&mut buffer)
            .map_err(|error| DistributionError::Io {
                path: source.to_path_buf(),
                source: error,
            })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        destination
            .write_all(&buffer[..count])
            .map_err(|error| DistributionError::Io {
                path: source.to_path_buf(),
                source: error,
            })?;
    }
    Ok(Sha256Digest::from_bytes(hasher.finalize().into()))
}

#[cfg(unix)]
fn add_owner_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path).map_err(|source| DistributionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(permissions.mode() | 0o100);
    fs::set_permissions(path, permissions).map_err(|source| DistributionError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(not(unix))]
fn add_owner_executable(_path: &Path) -> Result<()> {
    Ok(())
}

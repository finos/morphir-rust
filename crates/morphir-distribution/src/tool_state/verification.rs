//! Installed package manifest and content verification.

use super::package::{ToolPackageFile, VerifiedToolPackage};
use crate::store::{verify_executable_mode, verify_file};
use crate::{DistributionError, Result, Sha256Digest};
use morphir_common::home::MorphirHome;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn verify_package(home: &MorphirHome, package: &VerifiedToolPackage) -> Result<PathBuf> {
    verify_installed(home, package.store_path.as_path(), &package.files)
}

pub(super) fn verify_installed(
    home: &MorphirHome,
    store_path: &Path,
    files: &[ToolPackageFile],
) -> Result<PathBuf> {
    validate_manifest(store_path, files)?;
    let home_root = fs::canonicalize(home.root()).map_err(|source| DistributionError::Io {
        path: home.root().to_path_buf(),
        source,
    })?;
    let store_root =
        fs::canonicalize(home.tools_store_dir()).map_err(|source| DistributionError::Io {
            path: home.tools_store_dir(),
            source,
        })?;
    let requested = home.root().join(store_path);
    let program = fs::canonicalize(&requested).map_err(|source| DistributionError::Io {
        path: requested,
        source,
    })?;
    if !program.starts_with(&home_root) || !program.starts_with(&store_root) {
        return Err(DistributionError::InstalledPathEscape {
            path: program,
            root: home_root,
        });
    }
    for file in files {
        let path = home.root().join(file.path.as_path());
        let canonical =
            fs::canonicalize(&path).map_err(|source| DistributionError::Io { path, source })?;
        if !canonical.starts_with(&home_root) || !canonical.starts_with(&store_root) {
            return Err(DistributionError::InstalledPathEscape {
                path: canonical,
                root: home_root,
            });
        }
        verify_one_file(&canonical, &file.digest, file.length, file.executable)?;
    }
    Ok(program)
}

fn validate_manifest(store_path: &Path, files: &[ToolPackageFile]) -> Result<()> {
    if files.is_empty() {
        return Err(invalid_manifest("manifest cannot be empty"));
    }
    let unique = files.iter().map(|file| &file.path).collect::<BTreeSet<_>>();
    if unique.len() != files.len() {
        return Err(invalid_manifest("manifest paths must be unique"));
    }
    if !files
        .iter()
        .any(|file| file.path.as_path() == store_path && file.executable)
    {
        return Err(invalid_manifest(
            "manifest must contain the executable launch path",
        ));
    }
    Ok(())
}

fn invalid_manifest(reason: impl Into<String>) -> DistributionError {
    DistributionError::InvalidToolManifest {
        reason: reason.into(),
    }
}

pub(super) fn verify_one_file(
    path: &Path,
    digest: &Sha256Digest,
    length: u64,
    executable: bool,
) -> Result<()> {
    verify_file(path, digest)?;
    let actual = fs::metadata(path)
        .map_err(|source| DistributionError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    if actual != length {
        return Err(DistributionError::ToolLengthMismatch {
            path: path.to_path_buf(),
            expected: length,
            actual,
        });
    }
    verify_executable_mode(path, executable)
}

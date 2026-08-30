//! Installed package manifest and content verification.

use super::package::{ToolPackageFile, VerifiedToolPackage};
use crate::state_io::sync_parent_directory;
use crate::store::{verify_executable_mode, verify_file};
use crate::{DistributionError, RelativeArtifactPath, Result, Sha256Digest};
use morphir_common::home::MorphirHome;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn verify_package(home: &MorphirHome, package: &VerifiedToolPackage) -> Result<PathBuf> {
    verify_installed(
        home,
        package.store_path.as_path(),
        package
            .package_root
            .as_ref()
            .map(RelativeArtifactPath::as_path),
        &package.files,
    )
}

pub(super) fn sync_package(home: &MorphirHome, package: &VerifiedToolPackage) -> Result<()> {
    for file in &package.files {
        sync_installed_file(home, &home.root().join(file.path.as_path()))?;
    }
    Ok(())
}

pub(super) fn sync_installed_file(home: &MorphirHome, path: &Path) -> Result<()> {
    sync_file(path).map_err(|source| DistributionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let store = home.tools_store_dir();
    let mut child = path;
    while let Some(parent) = child.parent() {
        if !parent.starts_with(&store) {
            break;
        }
        sync_parent_directory(child)?;
        if parent == store {
            sync_parent_directory(parent)?;
            break;
        }
        child = parent;
    }
    Ok(())
}

#[cfg(windows)]
fn sync_file(path: &Path) -> std::io::Result<()> {
    fs::OpenOptions::new().write(true).open(path)?.sync_all()
}

#[cfg(not(windows))]
fn sync_file(path: &Path) -> std::io::Result<()> {
    fs::File::open(path)?.sync_all()
}

pub(super) fn verify_installed(
    home: &MorphirHome,
    store_path: &Path,
    package_root: Option<&Path>,
    files: &[ToolPackageFile],
) -> Result<PathBuf> {
    validate_manifest(store_path, package_root, files)?;
    verify_manifest_scope(home, package_root, files)?;
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

fn validate_manifest(
    store_path: &Path,
    package_root: Option<&Path>,
    files: &[ToolPackageFile],
) -> Result<()> {
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
    if let Some(package_root) = package_root
        && files
            .iter()
            .any(|file| !file.path.as_path().starts_with(package_root))
    {
        return Err(invalid_manifest(
            "every archived file must remain beneath the package root",
        ));
    }
    Ok(())
}

fn verify_manifest_scope(
    home: &MorphirHome,
    package_root: Option<&Path>,
    files: &[ToolPackageFile],
) -> Result<()> {
    let Some(package_root) = package_root else {
        return Ok(());
    };
    let root = home.root().join(package_root);
    let expected = files
        .iter()
        .map(|file| {
            file.path
                .as_path()
                .strip_prefix(package_root)
                .map(Path::to_path_buf)
                .map_err(|_| invalid_manifest("package file escaped its declared root"))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    let mut pending = vec![root.clone()];
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory).map_err(|source| DistributionError::Io {
            path: directory.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| DistributionError::Io {
                path: directory.clone(),
                source,
            })?;
            let path = entry.path();
            let relative = path
                .strip_prefix(&root)
                .expect("walked package entry remains beneath its root");
            let file_type = entry.file_type().map_err(|source| DistributionError::Io {
                path: path.clone(),
                source,
            })?;
            if file_type.is_symlink() {
                return Err(invalid_manifest(format!(
                    "package contains an unmanifested link: {}",
                    relative.display()
                )));
            }
            if file_type.is_dir() {
                pending.push(path);
            } else if !file_type.is_file() || !expected.contains(relative) {
                return Err(invalid_manifest(format!(
                    "package contains an unmanifested entry: {}",
                    relative.display()
                )));
            }
        }
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

//! Isolated materialization for raw executable and AppImage packages.

use super::package::{
    ToolPackageFile, VerifiedToolPackage, home_relative, package_from_resolved, portable_filename,
    verify_relative_package,
};
use super::package_key::extracted_package_path;
use super::verification::verify_one_file;
use crate::store::add_owner_executable;
use crate::{
    ArtifactFilename, ArtifactStore, DistributionError, DownloadedToolArtifact,
    RelativeArtifactPath, ResolvedTrustedToolArtifact, Result,
};
use morphir_common::home::MorphirHome;
use std::fs;
use std::path::Path;

pub(super) fn prepare(
    home: &MorphirHome,
    resolved: ResolvedTrustedToolArtifact,
    downloaded: DownloadedToolArtifact,
) -> Result<VerifiedToolPackage> {
    let downloaded = downloaded.into_path();
    let source_root = downloaded
        .parent()
        .expect("downloaded TUF target has a parent");
    let source_name = portable_filename(&downloaded)?;
    let filename = ArtifactFilename::parse(source_name)?;
    let source = RelativeArtifactPath::parse(source_name)?;
    let entry_point = resolved.artifact().launch().path();
    if entry_point.as_str() != source_name {
        return Err(DistributionError::ToolEntryPointMismatch {
            target: source_name.to_owned(),
            entry_point: entry_point.as_str().to_owned(),
        });
    }
    let stored = ArtifactStore::for_tools(home).materialize_file(
        source_root,
        &source,
        resolved.digest(),
        &filename,
        false,
    )?;
    verify_one_file(stored.path(), resolved.digest(), resolved.length(), false)?;

    let digest_directory = home.tools_store_dir().join(resolved.digest().to_string());
    let destination = extracted_package_path(&digest_directory, resolved.artifact());
    let package_directory = destination
        .parent()
        .expect("tool package destination has a parent");
    fs::create_dir_all(package_directory).map_err(|source| DistributionError::Io {
        path: package_directory.to_path_buf(),
        source,
    })?;
    let staging = tempfile::Builder::new()
        .prefix(".package-")
        .tempdir_in(package_directory)
        .map_err(|source| DistributionError::Io {
            path: package_directory.to_path_buf(),
            source,
        })?;
    let staging_root = staging.path().join("root");
    fs::create_dir(&staging_root).map_err(|source| DistributionError::Io {
        path: staging_root.clone(),
        source,
    })?;
    let staged_program = staging_root.join(entry_point.as_path());
    fs::copy(stored.path(), &staged_program).map_err(|source| DistributionError::Io {
        path: staged_program.clone(),
        source,
    })?;
    add_owner_executable(&staged_program)?;
    verify_one_file(&staged_program, resolved.digest(), resolved.length(), true)?;
    let relative_files = vec![ToolPackageFile {
        path: entry_point.clone(),
        digest: resolved.digest().clone(),
        length: resolved.length(),
        executable: true,
    }];
    publish(&staging_root, &destination, &relative_files)?;

    let program = destination.join(entry_point.as_path());
    let store_path = home_relative(home, &program)?;
    let package_root = home_relative(home, &destination)?;
    let files = vec![ToolPackageFile {
        path: store_path.clone(),
        digest: resolved.digest().clone(),
        length: resolved.length(),
        executable: true,
    }];
    Ok(package_from_resolved(
        resolved,
        store_path,
        Some(package_root),
        files,
        Vec::new(),
    ))
}

fn publish(staging_root: &Path, destination: &Path, files: &[ToolPackageFile]) -> Result<()> {
    if destination.exists() {
        return verify_relative_package(destination, files, &[]);
    }
    if let Err(source) = fs::rename(staging_root, destination) {
        if destination.exists() {
            verify_relative_package(destination, files, &[])
        } else {
            Err(DistributionError::Io {
                path: destination.to_path_buf(),
                source,
            })
        }
    } else {
        Ok(())
    }
}

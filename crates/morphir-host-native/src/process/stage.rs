//! Staging a verified extension executable to disk before it runs.

use crate::process::launch::ProcessProgram;
use morphir_host::HostError;
use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// Resolve a launch's program to an executable path, staging verified bytes if needed.
///
/// Returns the path to run and, for staged bytes, the temporary directory
/// that must outlive the child process.
pub async fn prepare_program(
    program: &ProcessProgram,
) -> Result<(PathBuf, Option<tempfile::TempDir>), HostError> {
    match program {
        ProcessProgram::Path(path) => Ok((path.clone(), None)),
        ProcessProgram::VerifiedBytes {
            filename,
            bytes,
            staging_directory,
        } => {
            let filename = filename.clone();
            let bytes = Arc::clone(bytes);
            let staging_directory = staging_directory.clone();
            tokio::task::spawn_blocking(move || {
                stage_verified_program(filename, bytes, staging_directory)
            })
            .await
            .map_err(|error| {
                HostError::Invalid(format!("Extension staging worker failed: {error}"))
            })?
        }
    }
}

fn stage_verified_program(
    filename: OsString,
    bytes: Arc<[u8]>,
    staging_directory: Option<PathBuf>,
) -> Result<(PathBuf, Option<tempfile::TempDir>), HostError> {
    validate_verified_program_filename(&filename)?;
    let mut builder = tempfile::Builder::new();
    builder.prefix("morphir-extension-");
    let directory = match staging_directory {
        Some(staging_directory) => {
            fs::create_dir_all(&staging_directory)?;
            builder.tempdir_in(staging_directory)?
        }
        None => builder.tempdir()?,
    };
    let path = directory.path().join(filename);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    std::io::Write::write_all(&mut file, &bytes)?;
    file.sync_all()?;
    make_owner_executable(&path)?;
    Ok((path, Some(directory)))
}

fn validate_verified_program_filename(filename: &OsStr) -> Result<(), HostError> {
    let mut components = Path::new(filename).components();
    if matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    ) {
        return Ok(());
    }
    Err(HostError::Invalid(
        "Verified extension executable must be a single filename".to_owned(),
    ))
}

#[cfg(unix)]
fn make_owner_executable(path: &Path) -> Result<(), HostError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).map_err(Into::into)
}

#[cfg(not(unix))]
fn make_owner_executable(_path: &Path) -> Result<(), HostError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::launch::ProcessLaunch;
    use morphir_extension_sdk::ExtensionInfo;

    #[tokio::test]
    async fn verified_bytes_stage_under_the_explicit_managed_directory() {
        let root = tempfile::tempdir().unwrap();
        let staging_directory = root.path().join("managed-staging");
        fs::create_dir(&staging_directory).unwrap();
        let launch = ProcessLaunch::from_verified_bytes_in(
            ExtensionInfo {
                id: "example".into(),
                ..ExtensionInfo::default()
            },
            OsStr::new("example"),
            b"#!/bin/sh\n",
            &staging_directory,
            root.path(),
        );

        let (program, _retained_directory) = prepare_program(launch.program()).await.unwrap();

        assert!(program.starts_with(&staging_directory));
    }

    #[test]
    fn verified_bytes_reject_path_components_without_writing_outside_staging() {
        let root = tempfile::tempdir().unwrap();
        let staging_directory = root.path().join("managed-staging");
        let absolute_escape = root.path().join("absolute-escape");

        for filename in [
            OsString::from("../relative-escape"),
            OsString::from("nested/escape"),
            absolute_escape.clone().into_os_string(),
        ] {
            let error = stage_verified_program(
                filename,
                Arc::from(&b"#!/bin/sh\n"[..]),
                Some(staging_directory.clone()),
            )
            .expect_err("verified process filename must be a single basename");

            assert!(error.to_string().contains("single filename"), "{error}");
        }
        assert!(!absolute_escape.exists());
        assert!(!staging_directory.join("relative-escape").exists());
        assert!(!staging_directory.join("nested/escape").exists());
    }
}

use super::{Error, require};
use crate::local_registry::RegistryPath;
use async_trait::async_trait;
use package_tough::{Transport, TransportError, TransportErrorKind, TransportStream};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub(super) fn absent(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
        Ok(_) => Err(Error::Refused("destination already exists")),
    }
}
pub(super) fn regular(path: &Path) -> Result<(), Error> {
    require(
        fs::symlink_metadata(path)?.file_type().is_file(),
        "expected a regular file, refusing symlinks",
    )
}
pub(super) fn directory(path: &Path) -> Result<PathBuf, Error> {
    require(
        fs::symlink_metadata(path)?.file_type().is_dir(),
        "expected a directory, refusing symlinks",
    )?;
    Ok(fs::canonicalize(path)?)
}
pub(super) fn safe(root: &Path, name: &str) -> Result<PathBuf, Error> {
    RegistryPath::parse(name).map_err(|_| Error::Refused("unsafe relative path"))?;
    let mut path = root.to_path_buf();
    let parts: Vec<_> = name.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        path.push(part);
        let ty = fs::symlink_metadata(&path)?.file_type();
        require(
            if i + 1 == parts.len() {
                ty.is_file()
            } else {
                ty.is_dir()
            },
            "unsafe registry file type",
        )?;
    }
    Ok(path)
}
pub(super) fn read(root: &Path, name: &str, limit: usize) -> Result<Vec<u8>, Error> {
    let path = safe(root, name)?;
    let file = File::open(path)?;
    require(
        file.metadata()?.len() <= limit as u64,
        "file resource limit",
    )?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    require(bytes.len() <= limit, "file resource limit")?;
    Ok(bytes)
}
pub(super) fn write(root: &Path, name: &str, bytes: &[u8]) -> Result<(), Error> {
    RegistryPath::parse(name).map_err(|_| Error::Refused("unsafe staged path"))?;
    let path = root.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
#[derive(Debug, Clone)]
pub(super) struct LocalTransport {
    pub root: PathBuf,
}
#[async_trait]
impl Transport for LocalTransport {
    async fn fetch(&self, url: url::Url) -> Result<TransportStream, TransportError> {
        let result = (|| -> Result<Vec<u8>, Error> {
            let path = url
                .to_file_path()
                .map_err(|_| Error::Refused("local files only"))?;
            // Windows canonicalization yields an extended prefix (\\?\C:\),
            // whereas file URLs decode to the ordinary drive/UNC spelling.
            // Normalize the comparison root through the same URL conversion;
            // retain the original canonical root for all filesystem reads.
            let comparison_root = url::Url::from_directory_path(&self.root)
                .map_err(|_| Error::Refused("registry path cannot be a file URL"))?
                .to_file_path()
                .map_err(|_| Error::Refused("local files only"))?;
            let relative = path
                .strip_prefix(&comparison_root)
                .map_err(|_| Error::Refused("metadata escaped registry"))?;
            let name = relative.to_str().ok_or(Error::Refused("non-UTF8 path"))?;
            #[cfg(windows)]
            let normalized = name.replace('\\', "/");
            #[cfg(windows)]
            let name = normalized.as_str();
            read(&self.root, name, 16_777_216)
        })();
        match result {
            Ok(bytes) => Ok(Box::pin(futures::stream::once(async {
                Ok(bytes::Bytes::from(bytes))
            }))),
            Err(e) => {
                let kind = match &e {
                    Error::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
                        TransportErrorKind::FileNotFound
                    }
                    _ => TransportErrorKind::Other,
                };
                Err(TransportError::new_with_cause(kind, url.as_str(), e))
            }
        }
    }
}
pub(super) fn inventory(root: &Path) -> Result<std::collections::BTreeSet<String>, Error> {
    fn visit(
        root: &Path,
        prefix: &str,
        found: &mut std::collections::BTreeSet<String>,
        entries: &mut usize,
    ) -> Result<(), Error> {
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            *entries += 1;
            require(*entries <= 8192, "bundle inventory limit")?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| Error::Refused("non-UTF8 bundle path"))?;
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            RegistryPath::parse(&relative).map_err(|_| Error::Refused("unsafe bundle path"))?;
            let ty = entry.file_type()?;
            if ty.is_dir() {
                let before = found.len();
                visit(&entry.path(), &relative, found, entries)?;
                require(
                    found.len() > before,
                    "bundle inventory contains empty directory",
                )?;
            } else {
                require(ty.is_file(), "unsafe bundle file type")?;
                found.insert(relative);
            }
        }
        Ok(())
    }
    let mut found = std::collections::BTreeSet::new();
    visit(root, "", &mut found, &mut 0)?;
    Ok(found)
}

pub(super) fn promote(source: &Path, destination: &Path) -> Result<(), Error> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let source = CString::new(source.as_os_str().as_bytes())
            .map_err(|_| Error::Refused("invalid output path"))?;
        let destination = CString::new(destination.as_os_str().as_bytes())
            .map_err(|_| Error::Refused("invalid output path"))?;
        // SAFETY: both NUL-terminated paths live through this call. Flags prohibit replacing any winner.
        let result = unsafe {
            #[cfg(target_os = "linux")]
            {
                libc::renameat2(
                    libc::AT_FDCWD,
                    source.as_ptr(),
                    libc::AT_FDCWD,
                    destination.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            }
            #[cfg(target_os = "macos")]
            {
                libc::renameatx_np(
                    libc::AT_FDCWD,
                    source.as_ptr(),
                    libc::AT_FDCWD,
                    destination.as_ptr(),
                    libc::RENAME_EXCL,
                )
            }
        };
        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        require(
            !source[..source.len() - 1].contains(&0)
                && !destination[..destination.len() - 1].contains(&0),
            "invalid output path",
        )?;
        // SAFETY: terminated UTF-16 strings remain live. No REPLACE_EXISTING flag is supplied.
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (source, destination);
        Err(Error::Refused("atomic no-replace publication unavailable"))
    }
}
#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[tokio::test]
    async fn canonical_windows_registry_preserves_missing_root_transport_status() {
        use package_tough::Transport;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("metadata")).unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let url = url::Url::from_directory_path(&root)
            .unwrap()
            .join("metadata/2.root.json")
            .unwrap();
        let error = super::LocalTransport { root }
            .fetch(url)
            .await
            .err()
            .unwrap();
        assert_eq!(
            error.kind(),
            package_tough::TransportErrorKind::FileNotFound
        );
    }
    #[test]
    fn promotion_never_overwrites_even_an_empty_competing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("stage");
        let destination = dir.path().join("output");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("ours"), b"ours").unwrap();
        std::fs::create_dir(&destination).unwrap();
        assert!(super::promote(&source, &destination).is_err());
        assert!(source.join("ours").is_file());
        assert_eq!(std::fs::read_dir(destination).unwrap().count(), 0);
    }
}

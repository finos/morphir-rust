use std::{
    ffi::{CStr, CString},
    fs::File,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    },
    path::{Component, Path},
};

/// A local APFS root resolved once, then traversed through directory handles.
pub(super) struct Directory(File);
fn name(value: &str) -> io::Result<CString> {
    if value.is_empty() || value == "." || value == ".." || value.contains('/') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a single safe path component",
        ));
    }
    CString::new(value).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))
}
fn result(value: i32) -> io::Result<i32> {
    if value < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(value)
    }
}
fn file_at(parent: i32, name: &CStr, flags: i32, mode: u32) -> io::Result<File> {
    // SAFETY: parent is an owned open directory descriptor and name is NUL terminated.
    let fd = result(unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode,
        )
    })?;
    // SAFETY: successful openat returned a new owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}
impl Directory {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        let absolute = std::fs::canonicalize(path)?;
        let mut directory = Self(File::open("/")?);
        for component in absolute.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(part) => {
                    let part = std::str::from_utf8(part.as_bytes()).map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidInput, "non-UTF8 path")
                    })?;
                    directory = directory.child(part)?;
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "non-anchored path",
                    ));
                }
            }
        }
        // SAFETY: stat points to initialized storage and the descriptor remains open.
        let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
        result(unsafe { libc::fstatfs(directory.0.as_raw_fd(), stat.as_mut_ptr()) })?;
        // SAFETY: fstatfs succeeded and initialized the entire struct.
        let stat = unsafe { stat.assume_init() };
        // SAFETY: f_fstypename is a fixed kernel-provided NUL-terminated string.
        let filesystem = unsafe { CStr::from_ptr(stat.f_fstypename.as_ptr()) };
        if filesystem.to_bytes() != b"apfs" || stat.f_flags & libc::MNT_LOCAL as u32 == 0 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "publication requires local APFS",
            ));
        }
        Ok(directory)
    }
    pub(super) fn child(&self, component: &str) -> io::Result<Self> {
        file_at(
            self.0.as_raw_fd(),
            &name(component)?,
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        )
        .map(Self)
    }
    pub(super) fn mkdir(&self, component: &str) -> io::Result<()> {
        let component = name(component)?;
        // SAFETY: valid anchored descriptor and name.
        result(unsafe { libc::mkdirat(self.0.as_raw_fd(), component.as_ptr(), 0o700) })?;
        self.flush()
    }
    pub(super) fn read(&self, component: &str, limit: usize) -> io::Result<Vec<u8>> {
        let file = file_at(
            self.0.as_raw_fd(),
            &name(component)?,
            libc::O_RDONLY | libc::O_NONBLOCK,
            0,
        )?;
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected a regular file with link count one",
            ));
        }
        let mut bytes = Vec::new();
        file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "resource limit",
            ));
        }
        Ok(bytes)
    }
    pub(super) fn install(&self, component: &str, bytes: &[u8]) -> io::Result<()> {
        let mut file = file_at(
            self.0.as_raw_fd(),
            &name(component)?,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )?;
        file.write_all(bytes)?;
        full_flush(&file)?;
        self.flush()
    }
    pub(super) fn replace(&self, prepared: &str, current: &str) -> io::Result<()> {
        let prepared = name(prepared)?;
        let current = name(current)?;
        // SAFETY: both names are single components under the same open directory.
        result(unsafe {
            libc::renameat(
                self.0.as_raw_fd(),
                prepared.as_ptr(),
                self.0.as_raw_fd(),
                current.as_ptr(),
            )
        })?;
        Ok(())
    }
    pub(super) fn flush(&self) -> io::Result<()> {
        full_flush(&self.0)
    }
}
fn full_flush(file: &File) -> io::Result<()> {
    file.sync_all()?;
    // SAFETY: F_FULLFSYNC takes no extra argument and file remains owned and open.
    result(unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) })?;
    Ok(())
}
impl Directory {
    pub(super) fn create(path: &Path) -> io::Result<Self> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing parent"))?;
        let component = path
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing name"))?;
        let parent = Self::open(parent)?;
        parent.mkdir(component)?;
        parent.child(component)
    }
    pub(super) fn identity(&self) -> io::Result<(u64, u64)> {
        use std::os::unix::fs::MetadataExt;
        let metadata = self.0.metadata()?;
        Ok((metadata.dev(), metadata.ino()))
    }
    pub(super) fn lock(&self) -> io::Result<File> {
        // Lock the anchored directory inode itself. No replaceable child
        // pathname can create a second lock identity for this registry.
        let file = self.0.try_clone()?;
        fs2::FileExt::lock_exclusive(&file)?;
        Ok(file)
    }
    pub(super) fn names(&self) -> io::Result<Vec<String>> {
        let file = file_at(
            self.0.as_raw_fd(),
            c".",
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        )?;
        use std::os::fd::IntoRawFd;
        let fd = file.into_raw_fd();
        // SAFETY: fd is an owned directory descriptor transferred to fdopendir.
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            // SAFETY: fdopendir did not take ownership on failure.
            unsafe { libc::close(fd) };
            return Err(io::Error::last_os_error());
        }
        struct Stream(*mut libc::DIR);
        impl Drop for Stream {
            fn drop(&mut self) {
                unsafe {
                    libc::closedir(self.0);
                }
            }
        }
        let stream = Stream(stream);
        let mut names = Vec::new();
        loop {
            // SAFETY: live, exclusively owned directory stream. Clear errno to distinguish EOF.
            unsafe {
                *libc::__error() = 0;
            }
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(0) {
                    return Err(error);
                }
                break;
            }
            // SAFETY: readdir returns a live entry with a NUL-terminated name until the next call.
            let entry = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
                .to_str()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-UTF8 entry"))?;
            if entry != "." && entry != ".." {
                names.push(entry.to_owned());
            }
            if names.len() > 100_000 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "directory entry limit",
                ));
            }
        }
        names.sort();
        Ok(names)
    }
    pub(super) fn promote(
        &self,
        source: &str,
        destination: &Directory,
        name_: &str,
    ) -> io::Result<()> {
        let source = name(source)?;
        let target = name(name_)?;
        // SAFETY: both parent descriptors and both single-component strings are live.
        result(unsafe {
            libc::renameatx_np(
                self.0.as_raw_fd(),
                source.as_ptr(),
                destination.0.as_raw_fd(),
                target.as_ptr(),
                libc::RENAME_EXCL,
            )
        })?;
        destination.flush()?;
        self.flush()
    }
    pub(super) fn read_path(&self, path: &str, limit: usize) -> io::Result<Vec<u8>> {
        let (parent, last) = path.rsplit_once('/').ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "expected directory path")
        })?;
        self.descend(parent)?.read(last, limit)
    }
    pub(super) fn descend(&self, path: &str) -> io::Result<Self> {
        let mut directory = Self(self.0.try_clone()?);
        for part in path.split('/') {
            directory = directory.child(part)?;
        }
        Ok(directory)
    }
    pub(super) fn install_path(
        &self,
        path: &str,
        bytes: &[u8],
        staging: &Directory,
    ) -> io::Result<()> {
        let (parent, last) = path.rsplit_once('/').ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "expected directory path")
        })?;
        let mut directory = Self(self.0.try_clone()?);
        for part in parent.split('/') {
            match directory.mkdir(part) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
            directory = directory.child(part)?;
        }
        match directory.read(last, bytes.len()) {
            Ok(existing) if existing == bytes => return Ok(()),
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "immutable-object-conflict",
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        staging.install(last, bytes)?;
        #[cfg(test)]
        super::checkpoint(super::FaultPoint::BeforeObjectPromotion)
            .map_err(|_| io::Error::other("injected pre-promotion failure"))?;
        staging.promote(last, &directory, last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anchored_storage_refuses_links_and_never_replaces_immutable_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let root = Directory::open(&temp.path().canonicalize().unwrap()).unwrap();
        root.mkdir("objects").unwrap();
        let objects = root.child("objects").unwrap();
        objects.install("one", b"first").unwrap();
        assert!(objects.install("one", b"second").is_err());
        assert_eq!(objects.read("one", 20).unwrap(), b"first");
        std::os::unix::fs::symlink("objects/one", temp.path().join("alias")).unwrap();
        assert!(root.read("alias", 20).is_err());
        assert!(root.read("objects/one", 20).is_err());
        assert!(root.read("../one", 20).is_err());
    }
    #[test]
    fn replacement_exposes_complete_bytes_and_flushes_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = Directory::open(&temp.path().canonicalize().unwrap()).unwrap();
        root.install("timestamp", b"old").unwrap();
        root.install("prepared", b"complete successor").unwrap();
        root.replace("prepared", "timestamp").unwrap();
        root.flush().unwrap();
        assert_eq!(root.read("timestamp", 100).unwrap(), b"complete successor");
    }
    #[test]
    fn immutable_reads_reject_hardlinks_to_mutable_outside_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = Directory::open(&temp.path().canonicalize().unwrap()).unwrap();
        root.install("source", b"mutable").unwrap();
        std::fs::hard_link(temp.path().join("source"), temp.path().join("alias")).unwrap();
        assert!(root.read("alias", 100).is_err());
        assert!(root.read("source", 100).is_err());
    }
    #[test]
    fn configured_root_alias_is_resolved_once_but_descendant_links_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, temp.path().join("alias")).unwrap();
        let root = Directory::open(&temp.path().join("alias")).unwrap();
        root.install("file", b"content").unwrap();
        std::os::unix::fs::symlink("file", real.join("link")).unwrap();
        assert!(root.read("link", 100).is_err());
    }
    #[test]
    fn failed_object_promotion_leaves_no_final_immutable_file() {
        let temp = tempfile::tempdir().unwrap();
        let root = Directory::open(&temp.path().canonicalize().unwrap()).unwrap();
        root.mkdir("staging").unwrap();
        let staging = root.child("staging").unwrap();
        super::super::FAULT.with(|fault| {
            fault.set(Some((
                super::super::FaultPoint::BeforeObjectPromotion,
                super::super::FaultAction::Fail,
            )))
        });
        let result = root.install_path("targets/records/item", b"complete bytes", &staging);
        super::super::FAULT.with(|fault| fault.set(None));
        assert!(result.is_err());
        assert!(!temp.path().join("targets/records/item").exists());
        assert_eq!(staging.read("item", 100).unwrap(), b"complete bytes");
    }
}

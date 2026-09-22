//! Experimental NTFS calls, only for trusted test directories. Not a durable provider.
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr,
};
use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_CREATE, FILE_DIRECTORY_FILE, FILE_MODE_INFORMATION, FILE_OPEN,
            FILE_SYNCHRONOUS_IO_NONALERT, FILE_WRITE_THROUGH, FileModeInformation, NtCreateFile,
            NtQueryInformationFile,
        },
    },
    Win32::{
        Foundation::{
            HANDLE, NTSTATUS, OBJ_CASE_INSENSITIVE, RtlNtStatusToDosError, UNICODE_STRING,
        },
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        Storage::FileSystem::*,
        System::{
            IO::IO_STATUS_BLOCK,
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    },
};

const DIRECTORY_ACCESS: u32 =
    DELETE | SYNCHRONIZE | FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES;
const DIRECTORY_OPTIONS: u32 =
    FILE_DIRECTORY_FILE | FILE_WRITE_THROUGH | FILE_SYNCHRONOUS_IO_NONALERT;

struct Directory(OwnedHandle);

fn nt_result(status: NTSTATUS) -> io::Result<()> {
    if status < 0 {
        // SAFETY: conversion accepts any NTSTATUS and owns no resources.
        Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ))
    } else {
        Ok(())
    }
}

impl Directory {
    fn open(
        root: HANDLE,
        name: &std::ffi::OsStr,
        disposition: u32,
        access: u32,
    ) -> io::Result<Self> {
        let mut wide: Vec<u16> = name.encode_wide().collect();
        let length = u16::try_from(wide.len() * 2)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "native name too long"))?;
        if wide.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "NUL in native name",
            ));
        }
        let name = UNICODE_STRING {
            Length: length,
            MaximumLength: length,
            Buffer: wide.as_mut_ptr(),
        };
        let attributes = OBJECT_ATTRIBUTES {
            Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: root,
            ObjectName: &name,
            Attributes: OBJ_CASE_INSENSITIVE,
            ..Default::default()
        };
        let mut handle = ptr::null_mut();
        let mut status = IO_STATUS_BLOCK::default();
        // SAFETY: live input buffers/parent handle and initialized writable outputs.
        // Synchronous options ensure completion before these stack values expire.
        nt_result(unsafe {
            NtCreateFile(
                &mut handle,
                access,
                &attributes,
                &mut status,
                ptr::null(),
                FILE_ATTRIBUTE_NORMAL,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                disposition,
                DIRECTORY_OPTIONS,
                ptr::null(),
                0,
            )
        })?;
        // SAFETY: successful synchronous NtCreateFile returned an owned handle.
        let directory = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
        directory.assert_write_through()?;
        Ok(directory)
    }

    fn root(path: &Path) -> io::Result<Self> {
        let path = path.canonicalize()?;
        let path = path.as_os_str().encode_wide().collect::<Vec<_>>();
        // canonicalize returns an extended DOS path. Refuse alternate namespaces.
        let prefix: Vec<u16> = r"\\?\".encode_utf16().collect();
        let tail = path
            .strip_prefix(prefix.as_slice())
            .ok_or_else(|| io::Error::other("expected canonical extended DOS path"))?;
        let nt_path: Vec<u16> = r"\??\".encode_utf16().chain(tail.iter().copied()).collect();
        use std::os::windows::ffi::OsStringExt;
        Self::open(
            ptr::null_mut(),
            &std::ffi::OsString::from_wide(&nt_path),
            FILE_OPEN,
            DIRECTORY_ACCESS,
        )
    }

    fn child(&self, name: &str, disposition: u32) -> io::Result<Self> {
        Self::open(
            self.0.as_raw_handle(),
            name.as_ref(),
            disposition,
            DIRECTORY_ACCESS,
        )
    }

    fn assert_write_through(&self) -> io::Result<()> {
        let mut mode = FILE_MODE_INFORMATION::default();
        let mut status = IO_STATUS_BLOCK::default();
        // SAFETY: valid handle; correctly sized/aligned writable output.
        nt_result(unsafe {
            NtQueryInformationFile(
                self.0.as_raw_handle(),
                &mut status,
                (&mut mode as *mut FILE_MODE_INFORMATION).cast(),
                size_of::<FILE_MODE_INFORMATION>() as u32,
                FileModeInformation,
            )
        })?;
        let required = FILE_WRITE_THROUGH | FILE_SYNCHRONOUS_IO_NONALERT;
        if mode.Mode & required != required {
            return Err(io::Error::other("directory handle lost required I/O mode"));
        }
        Ok(())
    }

    fn rename_to(&self, parent: &Self, name: &str) -> io::Result<()> {
        let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
        // Include the complete native structure, including trailing alignment,
        // plus the variable UTF-16 name required by the rename information contract.
        let bytes = size_of::<FILE_RENAME_INFO>() + wide.len() * 2;
        // FILE_RENAME_INFO elements guarantee native alignment and enough trailing storage.
        let mut buffer =
            vec![FILE_RENAME_INFO::default(); bytes.div_ceil(size_of::<FILE_RENAME_INFO>())];
        let info = buffer.as_mut_ptr();
        // SAFETY: aligned zeroed buffer large enough for header and entire UTF-16 name.
        // Handles and buffer remain live for the synchronous call; no replacement flag.
        unsafe {
            (*info).RootDirectory = parent.0.as_raw_handle();
            (*info).FileNameLength = ((wide.len() - 1) * 2) as u32;
            (*info).Anonymous.ReplaceIfExists = false;
            ptr::copy_nonoverlapping(
                wide.as_ptr(),
                ptr::addr_of_mut!((*info).FileName).cast(),
                wide.len(),
            );
            if SetFileInformationByHandle(
                self.0.as_raw_handle(),
                FileRenameInfo,
                info.cast(),
                bytes as u32,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

fn record_access_context() -> io::Result<()> {
    let mut token = ptr::null_mut();
    // SAFETY: current process pseudo-handle, query-only access and writable output.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful call returned an owned token handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0;
    // SAFETY: valid token and correctly sized/aligned output.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    eprintln!(
        "write-through candidate: directory access={DIRECTORY_ACCESS:#x}, options={DIRECTORY_OPTIONS:#x}, token_elevated={}; no privilege adjustment, backup intent, TxF or volume flush",
        elevation.TokenIsElevated
    );
    Ok(())
}

pub fn stage_tree(root: &Path) {
    let environment = super::filesystem::environment(root).unwrap();
    eprintln!("write-through candidate environment: {environment:?}");
    assert!(environment.required_local_filesystem(), "{environment:?}");
    record_access_context().unwrap();
    let root_handle = Directory::root(root).unwrap();
    let ancestors = root_handle.child("ancestors", FILE_CREATE).unwrap();
    let stage = ancestors.child("stage", FILE_CREATE).unwrap();
    stage.child("empty", FILE_CREATE).unwrap();
    let nested = stage.child("nested", FILE_CREATE).unwrap();
    nested.child("empty-leaf", FILE_CREATE).unwrap();
    for (name, bytes) in [
        ("payload", b"candidate payload".as_slice()),
        ("empty-file", b"".as_slice()),
    ] {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(FILE_FLAG_WRITE_THROUGH)
            .open(root.join("ancestors/stage/nested").join(name))
            .unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
    }
}

pub fn promote_tree(root: &Path) -> io::Result<()> {
    let root = Directory::root(root)?;
    let parent = root.child("ancestors", FILE_OPEN)?;
    let stage = parent.child("stage", FILE_OPEN)?;
    stage.rename_to(&parent, "winner")
}

pub fn assert_tree(root: &Path, name: &str) {
    let tree = root.join(name);
    assert!(tree.join("empty").is_dir());
    assert_eq!(fs::read_dir(tree.join("empty")).unwrap().count(), 0);
    assert!(tree.join("nested/empty-leaf").is_dir());
    assert_eq!(
        fs::read_dir(tree.join("nested/empty-leaf"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        fs::read(tree.join("nested/payload")).unwrap(),
        b"candidate payload"
    );
    assert_eq!(fs::read(tree.join("nested/empty-file")).unwrap(), b"");
}

#[test]
fn create_refuses_existing_directory_without_touching_contents() {
    let root = tempfile::tempdir().unwrap();
    let handle = Directory::root(root.path()).unwrap();
    handle.child("existing", FILE_CREATE).unwrap();
    fs::write(root.path().join("existing/preserved"), b"unchanged").unwrap();
    assert!(handle.child("existing", FILE_CREATE).is_err());
    assert_eq!(
        fs::read(root.path().join("existing/preserved")).unwrap(),
        b"unchanged"
    );
}

#[test]
fn rename_without_delete_access_fails_and_preserves_source() {
    let root = tempfile::tempdir().unwrap();
    stage_tree(root.path());
    let handle = Directory::root(root.path()).unwrap();
    let parent = handle.child("ancestors", FILE_OPEN).unwrap();
    let source = Directory::open(
        parent.0.as_raw_handle(),
        "stage".as_ref(),
        FILE_OPEN,
        DIRECTORY_ACCESS & !DELETE,
    )
    .unwrap();
    assert_eq!(
        source.rename_to(&parent, "winner").unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_tree(root.path(), "ancestors/stage");
    assert!(!root.path().join("ancestors/winner").exists());
}

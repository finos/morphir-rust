//! Owned identities used by every source observation comparison.
//!
//! The retained file pins the compared incarnation. Windows requires the full
//! FileIdInfo identity: legacy 64-bit file indices are not unique on ReFS.
use std::{fs::File, io, path::Path};

#[cfg(any(windows, test))]
#[derive(Debug, PartialEq, Eq)]
struct WindowsFileKey {
    volume_serial: u64,
    file_id: [u8; 16],
}

#[cfg(not(windows))]
#[derive(Debug, PartialEq, Eq)]
pub(super) struct FileIdentity(same_file::Handle);

#[cfg(not(windows))]
impl FileIdentity {
    pub(super) fn from_file(file: File) -> io::Result<Self> {
        same_file::Handle::from_file(file).map(Self)
    }

    pub(super) fn from_path(path: &Path) -> io::Result<Self> {
        same_file::Handle::from_path(path).map(Self)
    }
}

#[cfg(windows)]
pub(super) struct FileIdentity {
    // Kept through every equality comparison; copied identifiers alone do not
    // prevent the filesystem from reusing an identity after the file closes.
    file: File,
    key: WindowsFileKey,
}

#[cfg(windows)]
impl std::fmt::Debug for FileIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FileIdentity")
            .field("file", &self.file)
            .field("key", &self.key)
            .finish()
    }
}

#[cfg(windows)]
impl FileIdentity {
    pub(super) fn from_file(file: File) -> io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
        };

        let mut information = std::mem::MaybeUninit::<FILE_ID_INFO>::uninit();
        // SAFETY: the borrowed handle stays owned by `file`; the correctly
        // aligned output buffer has the exact size required by FileIdInfo.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                information.as_mut_ptr().cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        };
        if ok == 0 {
            // Never inspect failed output or substitute a zero/legacy key.
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful FileIdInfo initializes this complete structure.
        let information = unsafe { information.assume_init() };
        Ok(Self {
            file,
            key: WindowsFileKey {
                volume_serial: information.VolumeSerialNumber,
                file_id: information.FileId.Identifier,
            },
        })
    }

    pub(super) fn from_path(path: &Path) -> io::Result<Self> {
        // Preserve the existing metadata opener, including directory handling
        // and sharing. Its legacy key is never used for equality or fallback.
        let opened = same_file::Handle::from_path(path)?;
        Self::from_file(opened.as_file().try_clone()?)
    }
}

#[cfg(windows)]
impl PartialEq for FileIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

#[cfg(windows)]
impl Eq for FileIdentity {}

#[cfg(test)]
mod identity_tests;

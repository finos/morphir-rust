//! Owned filesystem identities for comparing already-authorized objects.
//!
//! Successful construction retains the genuine file owner until the identity
//! is dropped. Queries do not read, seek or decode content. They do not restrict
//! the operating-system access rights of the owned handle.
//!
//! Identity is separate from content revision, path spelling, permission,
//! workspace membership and confinement. Equal identities do not prove that
//! bytes remain unchanged, and a path label grants no authority.
//!
//! Windows compares the complete FileIdInfo volume and 128-bit file identity.
//! Other platforms retain existing same-file behavior. An unavailable query
//! returns an I/O error; no unknown, zero or legacy fallback is issued.
//! Debug shows the retained file diagnostic, omitting private identity keys.
//! Its text is unstable, not an identity token, contract or authority.
//!
//! ```no_run
//! use morphir_common::file_identity::FileIdentity;
//! use std::{fs::File, path::Path};
//! # fn main() -> std::io::Result<()> {
//! let path = Path::new("already-authorized-model.json");
//! let business_reader = File::open(path)?;
//! let held = FileIdentity::from_file(business_reader.try_clone()?)?;
//! let current = FileIdentity::from_path(path)?;
//! assert_eq!(held, current);
//! # Ok(()) }
//! ```
//!
//! The identity does not expose a content reader or a cloneable identifier.
//!
//! ```compile_fail
//! use morphir_common::file_identity::FileIdentity;
//! fn content_reader(identity: FileIdentity) {
//!     fn requires_read(_: impl std::io::Read) {}
//!     requires_read(identity);
//! }
//! ```
//!
//! ```compile_fail
//! use morphir_common::file_identity::FileIdentity;
//! fn copy_identifier(identity: &FileIdentity) -> FileIdentity {
//!     Clone::clone(identity)
//! }
//! ```
//!
//! ```compile_fail
//! use morphir_common::file_identity::FileIdentity;
//! fn serialize_identifier(identity: FileIdentity) {
//!     let _ = serde_json::to_value(identity);
//! }
//! ```
use std::{fs::File, io, path::Path};

/// Preserve existing read/follow access policy for opaque metadata use, while
/// excluding FIFO wait-for-peer and controlling-terminal acquisition on Unix.
pub(crate) fn metadata_query_options() -> cap_std::fs::OpenOptions {
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use cap_fs_ext::OpenOptionsSyncExt as _;
        use cap_std::fs::OpenOptionsExt as _;
        options.nonblock(true).custom_flags(libc::O_NOCTTY);
    }
    options
}

#[cfg(any(windows, test))]
#[derive(Debug, PartialEq, Eq)]
struct WindowsFileKey {
    volume_serial: u64,
    file_id: [u8; 16],
}

#[cfg(not(windows))]
#[derive(PartialEq, Eq)]
/// An owned filesystem identity with no exposed reader or primitive identifier.
///
/// The retained owner pins the compared incarnation. See the module documentation
/// for equality, query, diagnostic and authority boundaries.
pub struct FileIdentity(same_file::Handle);

#[cfg(not(windows))]
impl std::fmt::Debug for FileIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FileIdentity")
            .field("file", self.0.as_file())
            .finish()
    }
}

#[cfg(not(windows))]
impl FileIdentity {
    /// Consume an actual owned file and retain it after a successful query.
    /// This neither reads nor changes its file position. Failure drops the
    /// supplied owner; retain a genuine clone if a business reader must survive.
    pub fn from_file(file: File) -> io::Result<Self> {
        same_file::Handle::from_file(file).map(Self)
    }

    /// Open a path for an ambient identity query, including directories.
    /// This grants no confinement or transactional lookup guarantee. Prefer
    /// [`Self::from_file`] when an already-authorized owner is required.
    /// Unix query opens use NONBLOCK and NOCTTY and do not wait for a FIFO
    /// content peer. This is not a universal filesystem or network I/O deadline.
    pub fn from_path(path: &Path) -> io::Result<Self> {
        let opened = cap_std::fs::File::open_ambient_with(
            path,
            &metadata_query_options(),
            cap_std::ambient_authority(),
        )?;
        Self::from_file(opened.into_std())
    }
}

#[cfg(windows)]
/// An owned filesystem identity with no exposed reader or primitive identifier.
///
/// The retained owner pins the compared incarnation. See the module documentation
/// for equality, query, diagnostic and authority boundaries.
pub struct FileIdentity {
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
            .finish()
    }
}

#[cfg(windows)]
impl FileIdentity {
    /// Consume an actual owned file and retain it after a successful query.
    /// This neither reads nor changes its file position. Failure drops the
    /// supplied owner; retain a genuine clone if a business reader must survive.
    pub fn from_file(file: File) -> io::Result<Self> {
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

    /// Open a path for an ambient identity query, including directories.
    /// This preserves the existing platform opener and sharing policy. It grants
    /// no confinement or transactional lookup guarantee. Prefer
    /// [`Self::from_file`] when an already-authorized owner is required.
    pub fn from_path(path: &Path) -> io::Result<Self> {
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
#[path = "config/observation/identity/identity_tests.rs"]
mod identity_tests;

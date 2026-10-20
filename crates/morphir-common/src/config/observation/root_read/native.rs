//! Native single-root acquisition. Unsupported enforcement never falls back.
use super::RootReadError;
use cap_std::fs::Dir;
use std::{fs::File, path::Path};
#[cfg(all(test, target_os = "macos"))]
#[path = "darwin_support_tests.rs"]
mod support_tests;
#[derive(Clone, Copy)]
enum Access {
    MetadataFile,
    MetadataDirectory,
    Data,
}
pub(super) fn supported() -> bool {
    cfg!(any(target_os = "linux", target_os = "macos", windows))
}
#[cfg(not(target_os = "macos"))]
pub(super) fn ensure_support(_root: &Dir) -> Result<(), RootReadError> {
    if supported() {
        Ok(())
    } else {
        Err(RootReadError::UnsupportedPlatform)
    }
}
#[cfg(target_os = "macos")]
const O_RESOLVE_BENEATH: libc::c_int = 0x0000_1000;
#[cfg(target_os = "macos")]
const O_NOFOLLOW_ANY: libc::c_int = 0x2000_0000;
#[cfg(target_os = "macos")]
pub(super) fn ensure_support(root: &Dir) -> Result<(), RootReadError> {
    use std::os::fd::{AsRawFd, FromRawFd};
    // Pinned XNU vfs_lookup.c1440-1455: '..' at the starting vnode
    // returns ENOTCAPABLE before the filesystem-root special case.
    // Any other result cannot attest enforcement. This is metadata use only;
    // EVTONLY does not universally revoke kernel data access under every policy.
    let flags = libc::O_EVTONLY
        | libc::O_DIRECTORY
        | libc::O_CLOEXEC
        | libc::O_NONBLOCK
        | libc::O_NOCTTY
        | O_RESOLVE_BENEATH
        | O_NOFOLLOW_ANY;
    // SAFETY: live held directory, static terminated path, no create or Read.
    let fd = unsafe { libc::openat(root.as_raw_fd(), c"..".as_ptr(), flags) };
    if fd >= 0 {
        // SAFETY: close the unique unexpected descriptor before refusing.
        drop(unsafe { File::from_raw_fd(fd) });
        return Err(RootReadError::UnsupportedPlatform);
    }
    std::io::Error::last_os_error()
        .raw_os_error()
        .ok_or(RootReadError::UnsupportedPlatform)
        .and_then(probe_refusal)
}
#[cfg(target_os = "macos")]
fn probe_refusal(error: i32) -> Result<(), RootReadError> {
    if error == libc::ENOTCAPABLE {
        Ok(())
    } else {
        Err(RootReadError::UnsupportedPlatform)
    }
}
pub(super) fn metadata_directory(root: &Dir, path: &Path) -> Result<File, RootReadError> {
    acquire(root, path, Access::MetadataDirectory)
}
pub(super) fn metadata_file(root: &Dir, path: &Path) -> Result<File, RootReadError> {
    acquire(root, path, Access::MetadataFile)
}
pub(super) fn data_file(root: &Dir, path: &Path) -> Result<File, RootReadError> {
    acquire(root, path, Access::Data)
}
pub(super) fn regular(file: &File) -> Result<bool, RootReadError> {
    #[cfg(windows)]
    {
        windows::regular(file)
    }
    #[cfg(not(windows))]
    {
        Ok(file.metadata()?.is_file())
    }
}
#[cfg(target_os = "linux")]
fn acquire(root: &Dir, path: &Path, access: Access) -> Result<File, RootReadError> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    let path = c_string(path)?;
    let flags = OFlags::CLOEXEC
        | match access {
            Access::MetadataFile => OFlags::PATH,
            Access::MetadataDirectory => OFlags::PATH | OFlags::DIRECTORY,
            Access::Data => OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOCTTY,
        };
    openat2(
        root,
        path.as_c_str(),
        flags,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map(File::from)
    .map_err(|error| match error {
        rustix::io::Errno::NOSYS | rustix::io::Errno::INVAL | rustix::io::Errno::OPNOTSUPP => {
            RootReadError::UnsupportedPlatform
        }
        rustix::io::Errno::LOOP | rustix::io::Errno::XDEV | rustix::io::Errno::AGAIN => {
            RootReadError::ConfinementRefused
        }
        _ => RootReadError::Io(error.into()),
    })
}
#[cfg(target_os = "macos")]
fn acquire(root: &Dir, path: &Path, access: Access) -> Result<File, RootReadError> {
    use std::os::fd::{AsRawFd, FromRawFd};
    // XNU 12377.121.6 bsd/sys/fcntl.h. The locked libc lacks BENEATH.
    // O_NOFOLLOW_ANY must NOT be combined with legacy O_NOFOLLOW.
    let path = c_string(path)?;
    let flags = libc::O_CLOEXEC
        | libc::O_NONBLOCK
        | libc::O_NOCTTY
        | O_RESOLVE_BENEATH
        | O_NOFOLLOW_ANY
        | match access {
            // EVTONLY is used exclusively for metadata through an opaque owner.
            // It is not a claim that every kernel policy forbids data access.
            Access::MetadataFile => libc::O_EVTONLY,
            Access::MetadataDirectory => libc::O_EVTONLY | libc::O_DIRECTORY,
            Access::Data => libc::O_RDONLY,
        };
    // SAFETY: root is a live owned directory; CString is terminated and stable.
    // Only existing paths are opened; no mode/create/truncate/fallback applies.
    let fd = unsafe { libc::openat(root.as_raw_fd(), path.as_ptr(), flags) };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        return Err(match error.raw_os_error() {
            Some(libc::EINVAL | libc::ENOTSUP | libc::ENOSYS) => RootReadError::UnsupportedPlatform,
            Some(libc::ELOOP | libc::EXDEV | libc::EAGAIN | libc::ENOTCAPABLE) => {
                RootReadError::ConfinementRefused
            }
            _ => RootReadError::Io(error),
        });
    }
    // SAFETY: successful openat returned a new uniquely-owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn c_string(path: &Path) -> Result<std::ffi::CString, RootReadError> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    let capacity = bytes
        .len()
        .checked_add(1)
        .ok_or(RootReadError::InvalidRelativePath)?;
    let mut terminated = Vec::new();
    terminated
        .try_reserve_exact(capacity)
        .map_err(std::io::Error::other)?;
    terminated.extend_from_slice(bytes);
    terminated.push(0);
    std::ffi::CString::from_vec_with_nul(terminated).map_err(|_| RootReadError::InvalidRelativePath)
}
#[cfg(windows)]
#[path = "windows.rs"]
mod windows;
#[cfg(windows)]
fn acquire(root: &Dir, path: &Path, access: Access) -> Result<File, RootReadError> {
    windows::acquire(root, path, access)
}
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn acquire(_root: &Dir, _path: &Path, _access: Access) -> Result<File, RootReadError> {
    Err(RootReadError::UnsupportedPlatform)
}

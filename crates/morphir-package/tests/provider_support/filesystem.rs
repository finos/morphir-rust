//! Native calls for candidate qualification only, under trusted-directory assumptions.
use std::{fs, io, path::Path};

#[derive(Debug)]
pub struct Environment {
    pub os: &'static str,
    pub architecture: &'static str,
    pub filesystem: String,
    pub local: bool,
}
impl Environment {
    pub fn required_local_filesystem(&self) -> bool {
        self.local
            && matches!(
                (self.os, self.filesystem.as_str()),
                ("macos", "apfs") | ("linux", "ext4") | ("windows", "NTFS")
            )
    }
}
pub fn environment(path: &Path) -> io::Result<Environment> {
    let (filesystem, local) = native::filesystem(path)?;
    Ok(Environment {
        os: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        filesystem,
        local,
    })
}
pub fn promote(source: &Path, destination: &Path) -> io::Result<()> {
    native::promote(source, destination)
}
pub fn regular_single_link(path: &Path) -> io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Ok(false);
    }
    native::single_link(path, &metadata)
}

#[cfg(any(target_os = "linux", test))]
fn linux_mount_type(mount_id: u64, mounts: &str) -> io::Result<String> {
    let mut matches = mounts.lines().filter(|line| {
        line.split_whitespace()
            .next()
            .and_then(|id| id.parse::<u64>().ok())
            == Some(mount_id)
    });
    let entry = matches
        .next()
        .ok_or_else(|| io::Error::other("opened mount ID missing from mountinfo"))?;
    if matches.next().is_some() {
        return Err(io::Error::other("ambiguous mount ID in mountinfo"));
    }
    entry
        .split_once(" - ")
        .and_then(|(_, driver)| driver.split_whitespace().next())
        .map(str::to_owned)
        .ok_or_else(|| io::Error::other("mountinfo lacks filesystem type"))
}

#[cfg(test)]
mod mount_tests {
    use super::*;

    #[test]
    fn uses_active_mount_id_instead_of_a_covered_ext4_mount() {
        let mounts =
            "31 1 8:1 / /same rw - ext4 /dev/sda1 rw\n32 31 0:5 / /same rw - tmpfs tmpfs rw\n";
        assert_eq!(linux_mount_type(32, mounts).unwrap(), "tmpfs");
        assert_eq!(linux_mount_type(31, mounts).unwrap(), "ext4");
    }

    #[test]
    fn refuses_missing_or_ambiguous_mount_identity() {
        assert!(linux_mount_type(33, "31 1 8:1 / /same rw - ext4 /dev/sda1 rw\n").is_err());
        assert!(
            linux_mount_type(
                31,
                "31 1 8:1 / /same rw - ext4 /dev/sda1 rw\n31 1 0:5 / /same rw - tmpfs tmpfs rw\n"
            )
            .is_err()
        );
    }
}

#[cfg(unix)]
mod native {
    use super::*;
    #[cfg(target_os = "macos")]
    use std::ffi::CStr;
    use std::{
        ffi::CString,
        os::unix::{ffi::OsStrExt, fs::MetadataExt},
    };
    fn path_c(path: &Path) -> io::Result<CString> {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
    }
    pub fn promote(source: &Path, destination: &Path) -> io::Result<()> {
        let source = path_c(source)?;
        let destination = path_c(destination)?;
        // SAFETY: both strings are NUL-terminated and live for the call; flags forbid replacement.
        let result = unsafe {
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
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    pub fn single_link(_: &Path, metadata: &fs::Metadata) -> io::Result<bool> {
        Ok(metadata.nlink() == 1)
    }
    #[cfg(target_os = "macos")]
    pub fn filesystem(path: &Path) -> io::Result<(String, bool)> {
        let path = path_c(path)?;
        let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: statfs initializes the output on success.
        if unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful statfs initialized info, including a NUL-terminated name.
        let info = unsafe { info.assume_init() };
        let name = unsafe { CStr::from_ptr(info.f_fstypename.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        Ok((name, info.f_flags & libc::MNT_LOCAL as u32 != 0))
    }
    #[cfg(target_os = "linux")]
    pub fn filesystem(path: &Path) -> io::Result<(String, bool)> {
        use std::os::fd::AsRawFd;
        // Bind detection to the opened object. Longest-path matching can select a
        // covered mount, and statfs shares the same magic for ext2, ext3 and ext4.
        let directory = fs::File::open(path)?;
        let fdinfo = fs::read_to_string(format!("/proc/self/fdinfo/{}", directory.as_raw_fd()))?;
        let mount_id = fdinfo
            .lines()
            .find_map(|line| line.strip_prefix("mnt_id:"))
            .ok_or_else(|| io::Error::other("fdinfo lacks mount ID"))?
            .trim()
            .parse::<u64>()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mounts = fs::read_to_string("/proc/self/mountinfo")?;
        let name = linux_mount_type(mount_id, &mounts)?;
        let local = name == "ext4";
        Ok((name, local))
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::os::windows::{ffi::OsStrExt, fs::MetadataExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::*;
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;
    fn wide(path: &Path) -> io::Result<Vec<u16>> {
        let value: Vec<u16> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(io::Error::other("NUL in path"));
        }
        Ok(value.into_iter().chain([0]).collect())
    }
    pub fn promote(source: &Path, destination: &Path) -> io::Result<()> {
        let source = wide(source)?;
        let destination = wide(destination)?;
        // SAFETY: valid terminated paths. No replace, copy or reboot fallback is allowed.
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    pub fn single_link(path: &Path, metadata: &fs::Metadata) -> io::Result<bool> {
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Ok(false);
        }
        let file = fs::File::open(path)?;
        let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        // SAFETY: live file handle and writable output; output is read only on success.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { info.assume_init() }.nNumberOfLinks == 1)
    }
    pub fn filesystem(path: &Path) -> io::Result<(String, bool)> {
        let path = wide(&path.canonicalize()?)?;
        let mut volume = vec![0; 32768];
        let mut name = vec![0; 256];
        // SAFETY: all buffers are writable for their declared lengths; optional outputs are null.
        unsafe {
            if GetVolumePathNameW(path.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) == 0 {
                return Err(io::Error::last_os_error());
            }
            if GetVolumeInformationW(
                volume.as_ptr(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                name.as_mut_ptr(),
                name.len() as u32,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let local = GetDriveTypeW(volume.as_ptr()) == DRIVE_FIXED;
            let length = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            Ok((String::from_utf16_lossy(&name[..length]), local))
        }
    }
}

#[cfg(unix)]
pub fn flush(file: &fs::File) -> io::Result<()> {
    file.sync_all()?;
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: live descriptor; F_FULLFSYNC has no third argument.
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

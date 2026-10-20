//! Existing-only NT root-relative opens. No DOS/native fallback or raw ID API.
#[cfg(test)]
#[path = "windows_name_tests.rs"]
mod name_tests;
use super::{Access, RootReadError};
use cap_std::fs::Dir;
use std::{
    fs::File,
    io,
    mem::{MaybeUninit, size_of},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::Path,
    ptr,
};
use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_REPARSE_POINT,
            FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile,
        },
    },
    Win32::{
        Foundation::{OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING},
        Storage::FileSystem::{
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_READ_ATTRIBUTES,
            FILE_READ_DATA, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_DISK,
            FileAttributeTagInfo, GetFileInformationByHandleEx, GetFileType, SYNCHRONIZE,
        },
        System::IO::IO_STATUS_BLOCK,
    },
};

pub(super) fn acquire(root: &Dir, path: &Path, access: Access) -> Result<File, RootReadError> {
    let mut name = object_name(path)?;
    let bytes = u16::try_from(name.len() * 2).map_err(|_| RootReadError::InvalidRelativePath)?;
    let mut unicode = UNICODE_STRING {
        Length: bytes,
        MaximumLength: bytes,
        Buffer: name.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: root.as_raw_handle(),
        ObjectName: &mut unicode,
        Attributes: OBJ_DONT_REPARSE,
        SecurityDescriptor: ptr::null_mut(),
        SecurityQualityOfService: ptr::null_mut(),
    };
    let (rights, kind) = match access {
        Access::MetadataDirectory => (FILE_READ_ATTRIBUTES, FILE_DIRECTORY_FILE),
        Access::MetadataFile => (FILE_READ_ATTRIBUTES, FILE_NON_DIRECTORY_FILE),
        Access::Data => (
            FILE_READ_ATTRIBUTES | FILE_READ_DATA,
            FILE_NON_DIRECTORY_FILE,
        ),
    };
    let mut handle = ptr::null_mut();
    let mut status = MaybeUninit::<IO_STATUS_BLOCK>::uninit();
    // SAFETY: buffers and root remain live for this synchronous call. The
    // counted relative name has no NUL/prefix/stream or non-normal components.
    let result = unsafe {
        NtCreateFile(
            &mut handle,
            rights | SYNCHRONIZE,
            &attributes,
            status.as_mut_ptr(),
            ptr::null(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            kind | FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT,
            ptr::null(),
            0,
        )
    };
    if result < 0 {
        return Err(match result as u32 {
            0xC000_050B | 0x8000_002D | 0xC000_0279 => RootReadError::ConfinementRefused,
            0xC000_0002 | 0xC000_00BB | 0xC000_000D => RootReadError::UnsupportedPlatform,
            0xC000_00BA => RootReadError::NonRegular,
            _ => {
                // SAFETY: pure NT-status to DOS-code conversion.
                RootReadError::Io(io::Error::from_raw_os_error(unsafe {
                    RtlNtStatusToDosError(result)
                } as i32))
            }
        });
    }
    // SAFETY: successful NtCreateFile initialized one new uniquely-owned HANDLE.
    let file = unsafe { File::from_raw_handle(handle) };
    // A successful DATA handle must reach the shared Attempt before any
    // type/reparse/query refusal. Metadata acquisition issues no content event.
    if !matches!(access, Access::Data) {
        reject_reparse(&file)?;
    }
    Ok(file)
}
fn object_name(path: &Path) -> Result<Vec<u16>, RootReadError> {
    let units = path.as_os_str().encode_wide().count();
    units
        .checked_mul(2)
        .and_then(|n| u16::try_from(n).ok())
        .ok_or(RootReadError::InvalidRelativePath)?;
    let mut name = Vec::new();
    name.try_reserve_exact(units).map_err(io::Error::other)?;
    // The public operand already validated normal Windows path components.
    // NT names use backslash separators. Preserve all other code units, even
    // unpaired native surrogates; never apply DOS dot/space/case normalization.
    name.extend(
        path.as_os_str()
            .encode_wide()
            .map(|unit| if unit == 0x2f { 0x5c } else { unit }),
    );
    Ok(name)
}
fn reject_reparse(file: &File) -> Result<(), RootReadError> {
    let mut tag = MaybeUninit::<FILE_ATTRIBUTE_TAG_INFO>::uninit();
    // SAFETY: live file and correctly sized aligned output for this class.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            tag.as_mut_ptr().cast(),
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(RootReadError::Io(io::Error::last_os_error()));
    }
    // SAFETY: success initialized the complete attribute structure.
    if unsafe { tag.assume_init() }.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(RootReadError::ConfinementRefused);
    }
    Ok(())
}
pub(super) fn regular(file: &File) -> Result<bool, RootReadError> {
    reject_reparse(file)?;
    // SAFETY: file retains the live handle throughout query.
    let kind = unsafe { GetFileType(file.as_raw_handle()) };
    if kind == 0 {
        return Err(RootReadError::Io(io::Error::last_os_error()));
    }
    Ok(kind == FILE_TYPE_DISK && file.metadata()?.is_file())
}

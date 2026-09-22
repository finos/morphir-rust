//! Native observations for the test-only ordinary-user policy.
use super::{Identity, check_acl, check_executable_acl, check_identity};
use std::{
    ffi::c_void,
    io,
    mem::{offset_of, size_of},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{ERROR_INSUFFICIENT_BUFFER, HANDLE, LocalFree},
    Security::{Authorization::*, *},
    System::{
        SystemInformation::{IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64},
        Threading::{GetCurrentProcess, IsWow64Process2, OpenProcessToken},
    },
};

struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: each instance exclusively owns a Windows LocalAlloc result.
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn sid_string(sid: PSID) -> io::Result<String> {
    let mut output = ptr::null_mut();
    // SAFETY: caller supplies a SID inside a live native token/security buffer.
    if unsafe { ConvertSidToStringSidW(sid, &mut output) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _allocation = Local(output.cast());
    // SAFETY: successful conversion returns a NUL-terminated allocated UTF-16 string.
    unsafe {
        let mut length = 0;
        while *output.add(length) != 0 {
            length += 1;
        }
        Ok(String::from_utf16_lossy(std::slice::from_raw_parts(
            output, length,
        )))
    }
}

struct TokenBuffer {
    words: Vec<usize>,
    bytes: usize,
}
impl TokenBuffer {
    fn read(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: size query has no output buffer and a valid token.
        let status = unsafe { GetTokenInformation(token, class, ptr::null_mut(), 0, &mut bytes) };
        let error = io::Error::last_os_error();
        if status != 0
            || error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32)
            || bytes == 0
        {
            return Err(io::Error::other(format!(
                "token size query failed: class={class}, status={status}, bytes={bytes}, error={error}"
            )));
        }
        let mut words = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
        // SAFETY: aligned buffer owns at least the requested byte count.
        if unsafe {
            GetTokenInformation(token, class, words.as_mut_ptr().cast(), bytes, &mut bytes)
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            words,
            bytes: bytes as usize,
        })
    }
    fn header<T>(&self) -> &T {
        assert!(self.bytes >= size_of::<T>());
        // SAFETY: only native token header types, aligned within the returned buffer.
        unsafe { &*self.words.as_ptr().cast() }
    }
    fn array<T>(&self, offset: usize, count: usize) -> &[T] {
        assert!(offset + count.checked_mul(size_of::<T>()).unwrap() <= self.bytes);
        // SAFETY: native variable-length arrays with checked bounds and native offsets.
        unsafe {
            std::slice::from_raw_parts(self.words.as_ptr().cast::<u8>().add(offset).cast(), count)
        }
    }
}

fn fixed_elevation(token: HANDLE) -> io::Result<(TOKEN_ELEVATION, TOKEN_ELEVATION_TYPE)> {
    // These classes have fixed-size outputs. Microsoft's WIL likewise queries them
    // directly rather than relying on a zero-length size probe.
    let mut elevation = TOKEN_ELEVATION::default();
    let mut kind: TOKEN_ELEVATION_TYPE = 0;
    for (class, output, length) in [
        (
            TokenElevation,
            ptr::addr_of_mut!(elevation).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
        ),
        (
            TokenElevationType,
            ptr::addr_of_mut!(kind).cast(),
            size_of::<TOKEN_ELEVATION_TYPE>() as u32,
        ),
    ] {
        let mut returned = 0;
        // SAFETY: each output points to its class's correctly sized/aligned local
        // value, both of which remain live through these synchronous calls.
        if unsafe { GetTokenInformation(token, class, output, length, &mut returned) } == 0 {
            return Err(io::Error::other(format!(
                "fixed token query failed: class={class}, error={}",
                io::Error::last_os_error()
            )));
        }
        if returned != length {
            return Err(io::Error::other(format!(
                "unexpected fixed token output: class={class}, bytes={returned}, expected={length}"
            )));
        }
    }
    Ok((elevation, kind))
}

#[test]
fn fixed_elevation_queries_work_without_standard_user_fixture_setup() {
    let mut token = ptr::null_mut();
    // SAFETY: current process pseudo-handle, query-only access and valid output.
    assert_ne!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) },
        0
    );
    // SAFETY: OpenProcessToken succeeded and returned an owned handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let (elevation, kind) = fixed_elevation(token.as_raw_handle()).unwrap();
    assert!(elevation.TokenIsElevated <= 1);
    assert!(
        [
            TokenElevationTypeDefault,
            TokenElevationTypeFull,
            TokenElevationTypeLimited
        ]
        .contains(&kind)
    );
}

pub fn assert_identity(expected: &str) -> io::Result<()> {
    let mut token = ptr::null_mut();
    // SAFETY: current process pseudo-handle and writable token output; query access only.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful OpenProcessToken returned an owned handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let raw = token.as_raw_handle();
    let user = TokenBuffer::read(raw, TokenUser)?;
    let groups = TokenBuffer::read(raw, TokenGroups)?;
    let privileges = TokenBuffer::read(raw, TokenPrivileges)?;
    let mut enabled_privileges = Vec::new();
    for privilege in privileges.array::<LUID_AND_ATTRIBUTES>(
        offset_of!(TOKEN_PRIVILEGES, Privileges),
        privileges.header::<TOKEN_PRIVILEGES>().PrivilegeCount as usize,
    ) {
        let mut name = [0u16; 256];
        let mut length = name.len() as u32;
        // SAFETY: valid LUID and a writable UTF-16 output buffer of the declared length.
        if unsafe {
            LookupPrivilegeNameW(ptr::null(), &privilege.Luid, name.as_mut_ptr(), &mut length)
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let name = String::from_utf16_lossy(&name[..length as usize]);
        eprintln!(
            "ordinary-user token privilege: {name}, attributes={:#x}",
            privilege.Attributes
        );
        if privilege.Attributes & SE_PRIVILEGE_ENABLED != 0 {
            enabled_privileges.push(name);
        }
    }
    let (elevation, elevation_type) = fixed_elevation(raw)?;
    let identity = Identity {
        user: sid_string(user.header::<TOKEN_USER>().User.Sid)?,
        elevated: elevation.TokenIsElevated != 0,
        linked_token: elevation_type != TokenElevationTypeDefault,
        // SAFETY: live query token. Any restricting SID disqualifies this evidence.
        restricted: unsafe { IsTokenRestricted(raw) } != 0,
        groups: groups
            .array::<SID_AND_ATTRIBUTES>(
                offset_of!(TOKEN_GROUPS, Groups),
                groups.header::<TOKEN_GROUPS>().GroupCount as usize,
            )
            .iter()
            .map(|group| sid_string(group.Sid))
            .collect::<io::Result<_>>()?,
        enabled_privileges,
    };
    eprintln!("ordinary-user actual identity: {identity:?}");
    check_identity(&identity, expected)?;
    let expected_arch =
        std::env::var("MORPHIR_PROVIDER_EXPECTED_ARCH").map_err(io::Error::other)?;
    if expected_arch != std::env::consts::ARCH {
        return Err(io::Error::other("unexpected executable architecture"));
    }
    let mut process_machine = 0;
    let mut native_machine = 0;
    // SAFETY: current process handle and writable machine-code outputs.
    if unsafe {
        IsWow64Process2(
            GetCurrentProcess(),
            &mut process_machine,
            &mut native_machine,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let expected_machine = match expected_arch.as_str() {
        "x86_64" => IMAGE_FILE_MACHINE_AMD64,
        "aarch64" => IMAGE_FILE_MACHINE_ARM64,
        _ => return Err(io::Error::other("unsupported native Windows architecture")),
    };
    eprintln!(
        "ordinary-user native machines: process={process_machine:#x}, host={native_machine:#x}"
    );
    if process_machine != 0 || native_machine != expected_machine {
        return Err(io::Error::other(
            "ordinary-user probe must execute natively, not under emulation",
        ));
    }
    Ok(())
}

pub fn assert_acl(path: &Path, expected: &str, setup: &str) -> io::Result<()> {
    let (owner, allows) = read_acl(path)?;
    check_acl(&owner, &allows, expected, setup)
}

pub fn assert_executable_acl(path: &Path, expected: &str, setup: &str) -> io::Result<()> {
    let (owner, allows) = read_acl(path)?;
    check_executable_acl(&owner, &allows, expected, setup)
}

fn read_acl(path: &Path) -> io::Result<(String, Vec<(String, u32)>)> {
    let name: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut owner = ptr::null_mut();
    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: NUL-terminated fixture path; writable outputs remain valid until LocalFree.
    let status = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let _allocation = Local(descriptor);
    if dacl.is_null() {
        return Err(io::Error::other(
            "NULL fixture DACL grants unrestricted access",
        ));
    }
    let mut size = ACL_SIZE_INFORMATION::default();
    // SAFETY: live native ACL and correctly sized output.
    if unsafe {
        GetAclInformation(
            dacl,
            (&mut size as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut allows = Vec::new();
    for index in 0..size.AceCount {
        let mut ace = ptr::null_mut();
        // SAFETY: index is within the native ACL's reported entry count.
        if unsafe { GetAce(dacl, index, &mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: GetAce returns a live ACE starting with ACE_HEADER.
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        if header.AceType != 0 || usize::from(header.AceSize) < size_of::<ACCESS_ALLOWED_ACE>() {
            return Err(io::Error::other("unexpected fixture ACE type or size"));
        }
        // SAFETY: checked allow-ACE header and minimum structure size.
        let allowed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        let sid = sid_string(ptr::addr_of!(allowed.SidStart).cast_mut().cast())?;
        eprintln!(
            "ordinary-user ACL: path={}, sid={sid}, mask={:#x}, flags={:#x}",
            path.display(),
            allowed.Mask,
            header.AceFlags
        );
        // Inherit-only entries still must name an allowed principal, but do not
        // contribute to this object's effective user permissions.
        allows.push((
            sid,
            if u32::from(header.AceFlags) & INHERIT_ONLY_ACE == 0 {
                allowed.Mask
            } else {
                0
            },
        ));
    }
    let owner = sid_string(owner)?;
    eprintln!(
        "ordinary-user ACL owner: path={}, sid={owner}",
        path.display()
    );
    Ok((owner, allows))
}

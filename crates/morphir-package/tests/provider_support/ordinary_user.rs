//! Test-only ordinary-user evidence policy; native observations are supplied separately.
use std::io;

#[derive(Clone, Debug)]
struct Identity {
    user: String,
    elevated: bool,
    linked_token: bool,
    restricted: bool,
    groups: Vec<String>,
    enabled_privileges: Vec<String>,
}

fn check_identity(identity: &Identity, expected: &str) -> io::Result<()> {
    if identity.user != expected
        || identity.elevated
        || identity.linked_token
        || identity.restricted
        || identity.groups.iter().any(|sid| sid == "S-1-5-32-544")
        || !identity.groups.iter().any(|sid| sid == "S-1-5-32-545")
    {
        return Err(io::Error::other(
            "not the required unrestricted standard-user token",
        ));
    }
    for privilege in &identity.enabled_privileges {
        if !matches!(
            privilege.as_str(),
            "SeChangeNotifyPrivilege"
                | "SeShutdownPrivilege"
                | "SeUndockPrivilege"
                | "SeIncreaseWorkingSetPrivilege"
                | "SeTimeZonePrivilege"
        ) {
            return Err(io::Error::other(format!(
                "unexpected enabled privilege: {privilege}"
            )));
        }
    }
    Ok(())
}

fn check_acl(owner: &str, allows: &[(String, u32)], expected: &str, setup: &str) -> io::Result<()> {
    let mut user_access = 0;
    for (sid, mask) in allows {
        if sid == expected {
            user_access |= mask;
        } else if sid != "S-1-5-18" && sid != setup {
            return Err(io::Error::other(format!(
                "unexpected fixture ACL principal: {sid}"
            )));
        }
    }
    if owner != expected || user_access & 0x1f01ff != 0x1f01ff {
        return Err(io::Error::other(
            "fixture is not owned and fully controlled by the standard user",
        ));
    }
    Ok(())
}

fn check_executable_acl(
    owner: &str,
    allows: &[(String, u32)],
    expected: &str,
    setup: &str,
) -> io::Result<()> {
    let mut user_access = 0;
    for (sid, mask) in allows {
        if sid == expected {
            user_access |= mask;
        } else if sid != "S-1-5-18" && sid != setup {
            return Err(io::Error::other(
                "unexpected executable-boundary ACL principal",
            ));
        }
    }
    // FILE_GENERIC_READ | FILE_GENERIC_EXECUTE, with no write/delete/ACL rights.
    if owner != setup || user_access != 0x1200a9 {
        return Err(io::Error::other(
            "executable boundary must be setup-owned and user read/execute only",
        ));
    }
    Ok(())
}

#[cfg(windows)]
#[path = "ordinary_user_windows.rs"]
mod native;

#[cfg(windows)]
pub fn assert_if_requested(path: &std::path::Path) {
    if std::env::var_os("MORPHIR_PROVIDER_STANDARD_USER").is_some() {
        assert_eq!(
            std::env::var("MORPHIR_PROVIDER_STANDARD_USER").unwrap(),
            "1"
        );
        assert_required(path);
    }
}

#[cfg(windows)]
pub fn assert_required(path: &std::path::Path) {
    assert_eq!(
        std::env::var("MORPHIR_PROVIDER_STANDARD_USER").unwrap(),
        "1"
    );
    let expected = std::env::var("MORPHIR_PROVIDER_EXPECTED_SID").unwrap();
    let setup = std::env::var("MORPHIR_PROVIDER_SETUP_SID").unwrap();
    assert_ne!(expected, setup);
    native::assert_identity(&expected).unwrap();
    native::assert_acl(path, &expected, &setup).unwrap();
}

#[cfg(windows)]
pub fn assert_executable_boundary() {
    let expected = std::env::var("MORPHIR_PROVIDER_EXPECTED_SID").unwrap();
    let setup = std::env::var("MORPHIR_PROVIDER_SETUP_SID").unwrap();
    let executable = std::env::current_exe().unwrap();
    for path in [executable.as_path(), executable.parent().unwrap()] {
        native::assert_executable_acl(path, &expected, &setup).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const USER: &str = "S-1-5-21-1-2-3-1001";
    const SETUP: &str = "S-1-5-21-1-2-3-1000";
    const FULL_CONTROL: u32 = 0x1f01ff;
    fn standard_user() -> Identity {
        Identity {
            user: USER.into(),
            elevated: false,
            linked_token: false,
            restricted: false,
            groups: vec!["S-1-5-32-545".into()],
            enabled_privileges: vec!["SeChangeNotifyPrivilege".into()],
        }
    }
    #[test]
    fn filtered_administrator_is_not_standard_user_evidence() {
        let mut identity = standard_user();
        identity.groups.push("S-1-5-32-544".into());
        assert!(check_identity(&identity, USER).is_err());
    }
    #[test]
    fn unexpected_identity_elevation_restriction_or_privilege_fails() {
        assert!(check_identity(&standard_user(), SETUP).is_err());
        for field in ["elevated", "linked", "restricted", "privilege"] {
            let mut identity = standard_user();
            match field {
                "elevated" => identity.elevated = true,
                "linked" => identity.linked_token = true,
                "restricted" => identity.restricted = true,
                _ => identity.enabled_privileges.push("SeBackupPrivilege".into()),
            }
            assert!(check_identity(&identity, USER).is_err(), "{field}");
        }
        check_identity(&standard_user(), USER).unwrap();
    }
    #[test]
    fn executable_boundary_requires_setup_owner_and_exact_read_execute_access() {
        let allows = vec![
            (USER.into(), 0x1200a9),
            ("S-1-5-18".into(), FULL_CONTROL),
            (SETUP.into(), FULL_CONTROL),
        ];
        check_executable_acl(SETUP, &allows, USER, SETUP).unwrap();
        assert!(check_executable_acl(USER, &allows, USER, SETUP).is_err());
        for access in [0, 0x1200a9 | 2, FULL_CONTROL] {
            let mut changed = allows.clone();
            changed[0].1 = access;
            assert!(check_executable_acl(SETUP, &changed, USER, SETUP).is_err());
        }
        let mut broad = allows;
        broad.push(("S-1-5-32-545".into(), 2));
        assert!(check_executable_acl(SETUP, &broad, USER, SETUP).is_err());
    }
    #[test]
    fn fixture_acl_requires_owner_access_and_excludes_other_principals() {
        let allows = vec![
            (USER.into(), FULL_CONTROL),
            ("S-1-5-18".into(), FULL_CONTROL),
            (SETUP.into(), FULL_CONTROL),
        ];
        check_acl(USER, &allows, USER, SETUP).unwrap();
        assert!(check_acl(SETUP, &allows, USER, SETUP).is_err());
        assert!(check_acl(USER, &allows[1..], USER, SETUP).is_err());
        let mut broad = allows.clone();
        broad.push(("S-1-1-0".into(), 2));
        assert!(check_acl(USER, &broad, USER, SETUP).is_err());
    }
}

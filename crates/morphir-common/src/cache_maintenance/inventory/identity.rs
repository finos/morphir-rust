use crate::cache_maintenance::model::valid_entry_path;
use std::ffi::OsStr;
use std::path::Path;

pub(super) fn portable_identity(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(portable_component(value)),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn portable_component(value: &OsStr) -> String {
    if let Some(value) = value.to_str() {
        if valid_entry_path(value) {
            return value.to_owned();
        }
        return value
            .as_bytes()
            .iter()
            .map(|byte| format!("%{byte:02X}"))
            .collect();
    }
    portable_non_unicode_component(value)
}

#[cfg(unix)]
fn portable_non_unicode_component(value: &OsStr) -> String {
    use std::os::unix::ffi::OsStrExt;
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("%{byte:02X}"))
        .collect()
}

#[cfg(windows)]
fn portable_non_unicode_component(value: &OsStr) -> String {
    use std::os::windows::ffi::OsStrExt;
    value
        .encode_wide()
        .map(|unit| format!("%u{unit:04X}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::portable_component;
    use std::ffi::OsStr;

    #[test]
    fn nonportable_observed_separators_use_protected_identities() {
        assert_eq!(portable_component(OsStr::new("a:b")), "%61%3A%62");
        assert_eq!(portable_component(OsStr::new(r"a\b")), "%61%5C%62");
    }
}

//! Private NT-name controls; these do not qualify a Windows acquisition.
use super::*;
#[test]
fn validated_windows_separators_become_nt_separators_only() {
    let expected: Vec<u16> = b"compile.dest\\model.json"
        .iter()
        .map(|byte| u16::from(*byte))
        .collect();
    assert_eq!(
        object_name(Path::new("compile.dest/model.json")).unwrap(),
        expected
    );
    assert_eq!(
        object_name(Path::new("compile.dest\\model.json")).unwrap(),
        expected
    );
    assert_eq!(
        object_name(Path::new("a/b\\c")).unwrap(),
        [0x61, 0x5c, 0x62, 0x5c, 0x63]
    );
}
#[test]
fn normal_component_unicode_and_case_are_preserved_exactly() {
    assert_eq!(
        object_name(Path::new("é/😀.json")).unwrap(),
        [
            0x00e9, 0x005c, 0xd83d, 0xde00, 0x002e, 0x006a, 0x0073, 0x006f, 0x006e
        ]
    );
    assert_eq!(
        object_name(Path::new("A/Z")).unwrap(),
        [0x0041, 0x005c, 0x005a]
    );
    assert_eq!(
        object_name(Path::new("A./B ")).unwrap(),
        [0x41, 0x2e, 0x5c, 0x42, 0x20]
    );
}
#[test]
fn native_unpaired_codeunits_survive_the_validated_path_boundary() {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    let native = OsString::from_wide(&[0xd800, 0x2f, 0xdc00]);
    crate::config::observation::RootRelativeFile::parse(Path::new(&native)).unwrap();
    assert_eq!(
        object_name(Path::new(&native)).unwrap(),
        [0xd800, 0x5c, 0xdc00]
    );
}
#[test]
fn counted_nt_name_limit_is_utf16_bytes_not_scalar_or_utf8_count() {
    assert_eq!(
        object_name(Path::new(&"a".repeat(32767))).unwrap().len(),
        32767
    );
    for path in ["a".repeat(32768), "😀".repeat(16384)] {
        assert!(matches!(
            object_name(Path::new(&path)),
            Err(RootReadError::InvalidRelativePath)
        ));
    }
}

//! Unit child of private observation::identity. These raw keys are equality
//! fixtures ONLY, never issued identities, grants, observed events or collisions.
use super::WindowsFileKey;

fn key(volume_serial: u64, file_id: [u8; 16]) -> WindowsFileKey {
    WindowsFileKey {
        volume_serial,
        file_id,
    }
}

#[test]
fn identical_full_volume_and_128_bit_key_compare_equal() {
    let bytes = [
        0x10, 0x21, 0x32, 0x43, 0x54, 0x65, 0x76, 0x87, 0x98, 0xa9, 0xba, 0xcb, 0xdc, 0xed, 0xfe,
        0x0f,
    ];
    let first = key(0x1234_5678_9abc_def0, bytes);
    let second = key(0x1234_5678_9abc_def0, bytes);
    let third = key(0x1234_5678_9abc_def0, bytes);
    assert!(first == second);
    assert!(second == first);
    assert!(second == third);
    assert!(first == third);
}

#[test]
fn every_one_of_128_file_identifier_bits_participates_in_equality() {
    // Independent all-zero vs each fixed one-bit vector. No runtime file ID or
    // implementation result supplies the expected inequality.
    for byte in 0..16 {
        for bit in 0..8 {
            let mut different = [0_u8; 16];
            different[byte] = 1 << bit;
            let first = key(7, [0; 16]);
            let second = key(7, different);
            assert!(first != second, "file identity byte {byte}, bit {bit}");
            assert!(second != first, "symmetric byte {byte}, bit {bit}");
        }
    }
}

#[test]
fn same_legacy_low64_and_different_high64_are_distinct() {
    let first = key(
        0x0102_0304_0506_0708,
        [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
    );
    let second = key(
        0x0102_0304_0506_0708,
        [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0, 0, 0, 0, 0, 0, 0, 0x80,
        ],
    );
    assert!(first != second);
}

#[test]
fn every_one_of_64_volume_bits_participates_in_equality() {
    for bit in 0..64 {
        assert!(key(0, [0x5a; 16]) != key(1_u64 << bit, [0x5a; 16]));
    }
}

#[test]
fn zero_key_fixture_is_only_equality_data_not_a_query_failure_substitute() {
    let first = key(0, [0; 16]);
    let second = key(0, [0; 16]);
    assert!(first == second);
    assert!(key(0, [0; 16]) != key(1, [0; 16]));
    assert!(key(0, [0; 16]) != key(0, [1; 16]));
    // An actual failed query must never reach this successful-key domain.
}

#[cfg(not(windows))]
#[test]
fn nonwindows_adapter_preserves_existing_actual_same_file_semantics() {
    use super::FileIdentity;
    let parent = tempfile::tempdir().unwrap();
    let first = parent.path().join("first");
    let second = parent.path().join("second");
    let link = parent.path().join("same-file-hardlink");
    std::fs::write(&first, b"same bytes").unwrap();
    std::fs::write(&second, b"same bytes").unwrap();
    std::fs::hard_link(&first, &link).unwrap();
    let owned = FileIdentity::from_file(std::fs::File::open(&first).unwrap()).unwrap();
    let same_path = FileIdentity::from_path(&first).unwrap();
    let same_link = FileIdentity::from_path(&link).unwrap();
    let distinct = FileIdentity::from_path(&second).unwrap();
    assert!(owned == same_path);
    assert!(owned == same_link);
    assert!(owned != distinct);
    assert!(
        same_file::Handle::from_path(&first).unwrap()
            == same_file::Handle::from_path(&link).unwrap()
    );
    assert!(
        same_file::Handle::from_path(&first).unwrap()
            != same_file::Handle::from_path(&second).unwrap()
    );
}

#[cfg(windows)]
mod actual_windows {
    use super::{WindowsFileKey, key};
    use crate::config::observation::identity::FileIdentity;
    use std::{
        fs::File,
        io::{Read as _, Seek as _, Write as _},
        os::windows::io::{AsRawHandle as _, OwnedHandle},
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
    };

    const PRIMARY: &[u8] =
        include_bytes!("../../../../tests/source_observation_fixtures/primary.json");

    fn actual_key(file: &File) -> WindowsFileKey {
        let mut information = std::mem::MaybeUninit::<FILE_ID_INFO>::uninit();
        let ok = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                information.as_mut_ptr().cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        };
        assert_ne!(
            ok,
            0,
            "actual FileIdInfo query: {}",
            std::io::Error::last_os_error()
        );
        let information = unsafe { information.assume_init() };
        key(
            information.VolumeSerialNumber,
            information.FileId.Identifier,
        )
    }

    #[test]
    fn actual_file_owner_stores_full_native_query_without_consuming_content() {
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("model.json");
        std::fs::write(&path, PRIMARY).unwrap();
        let file = File::open(&path).unwrap();
        let mut business_reader = file.try_clone().unwrap();
        let expected = actual_key(&file);
        let original_handle = file.as_raw_handle();
        let identity = FileIdentity::from_file(file).unwrap();
        assert!(identity.key == expected);
        assert_eq!(identity.file.as_raw_handle(), original_handle);
        assert_eq!(identity.file.metadata().unwrap().len(), 111);
        assert_eq!(business_reader.stream_position().unwrap(), 0);
        let mut bytes = Vec::new();
        business_reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, PRIMARY);
        let same = FileIdentity::from_path(&path).unwrap();
        assert!(identity == same);
        // The reads above are explicit test business-reader calibration, not
        // source-observer events or a metadata-read permission promise.
    }

    #[test]
    fn retained_owner_pins_actual_original_while_same_path_is_replaced() {
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("model.json");
        let old_path = parent.path().join("retained-original.json");
        std::fs::write(&path, PRIMARY).unwrap();
        let identity = FileIdentity::from_path(&path).unwrap();
        let before = actual_key(&identity.file);
        std::fs::rename(&path, &old_path).unwrap();
        std::fs::write(&path, PRIMARY).unwrap();
        let replacement = FileIdentity::from_path(&path).unwrap();
        let original_again = FileIdentity::from_path(&old_path).unwrap();
        assert!(identity.key == before);
        assert!(actual_key(&identity.file) == before);
        assert!(identity == original_again);
        assert!(identity != replacement);
        // Rust File ownership pins the original incarnation. We do not claim
        // an actual ID-reuse collision or query a stale raw handle after drop.
    }

    #[test]
    fn actual_directory_and_owned_clone_compare_by_full_file_id_info() {
        let parent = tempfile::tempdir().unwrap();
        let root = cap_std::fs::Dir::open_ambient_dir(parent.path(), cap_std::ambient_authority())
            .unwrap();
        let identity = FileIdentity::from_file(root.try_clone().unwrap().into_std_file()).unwrap();
        let native = actual_key(&identity.file);
        let same = FileIdentity::from_path(parent.path()).unwrap();
        assert!(identity.key == native);
        assert!(identity == same);
    }

    #[test]
    fn actual_pipe_query_failure_is_error_not_zero_or_legacy_identity() {
        // Genuine Windows CreatePipe via std, no forged invalid HANDLE and no
        // mocked query result. FILE_ID_INFO does not accept pipe handles.
        let (reader, mut writer) = std::io::pipe().unwrap();
        let mut business_reader = reader.try_clone().unwrap();
        let file = File::from(OwnedHandle::from(reader));
        assert!(FileIdentity::from_file(file).is_err());
        // Failure retires only the consumed identity clone. A separate real
        // business reader still works; no fallback identity is produced.
        writer.write_all(b"x").unwrap();
        let mut byte = [0_u8; 1];
        business_reader.read_exact(&mut byte).unwrap();
        assert_eq!(&byte, b"x");
    }
}

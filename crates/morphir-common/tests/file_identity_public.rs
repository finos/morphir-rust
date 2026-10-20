//! Prospective external controls, frozen before public API implementation.
use cap_std::{ambient_authority, fs::Dir};
use morphir_common::file_identity::FileIdentity;
use std::{
    fs::File,
    io::{Read as _, Seek as _, SeekFrom},
};

const PRIMARY: &[u8] = include_bytes!("source_observation_fixtures/primary.json");
const PRIVATE: &[u8] = include_bytes!("source_observation_fixtures/private-record.json");

#[test]
fn diagnostic_output_does_not_serialize_the_private_windows_identity_key() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("model.json");
    std::fs::write(&path, PRIMARY).unwrap();
    let identity = FileIdentity::from_file(File::open(&path).unwrap()).unwrap();
    let diagnostic = format!("{identity:?}");
    assert!(!diagnostic.contains("WindowsFileKey"));
    assert!(!diagnostic.contains("volume_serial:"));
    assert!(!diagnostic.contains("file_id:"));
}

#[test]
fn owned_file_query_preserves_reader_position_and_exact_original_bytes() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("model.json");
    std::fs::write(&path, PRIMARY).unwrap();
    let mut reader = File::open(&path).unwrap();
    reader.seek(SeekFrom::Start(7)).unwrap();
    let identity = FileIdentity::from_file(reader.try_clone().unwrap()).unwrap();
    let named = FileIdentity::from_path(&path).unwrap();
    assert_eq!(identity, named);
    assert_eq!(reader.stream_position().unwrap(), 7);
    let mut actual = Vec::new();
    reader.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, PRIMARY[7..]);
}

#[test]
fn hardlinks_match_but_distinct_files_with_the_same_bytes_do_not() {
    let fixture = tempfile::tempdir().unwrap();
    let first = fixture.path().join("first.json");
    let second = fixture.path().join("same-bytes.json");
    let alias = fixture.path().join("hardlink.json");
    std::fs::write(&first, PRIMARY).unwrap();
    std::fs::write(&second, PRIMARY).unwrap();
    std::fs::hard_link(&first, &alias).unwrap();
    let owned = FileIdentity::from_file(File::open(&first).unwrap()).unwrap();
    assert_eq!(owned, FileIdentity::from_path(&alias).unwrap());
    assert_ne!(owned, FileIdentity::from_path(&second).unwrap());
}

#[test]
fn actual_directory_owned_clone_and_path_query_identify_the_same_directory() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let directory = Dir::open_ambient_dir(first.path(), ambient_authority()).unwrap();
    let owned = FileIdentity::from_file(directory.try_clone().unwrap().into_std_file()).unwrap();
    let cloned = FileIdentity::from_file(directory.try_clone().unwrap().into_std_file()).unwrap();
    assert_eq!(owned, cloned);
    assert_eq!(owned, FileIdentity::from_path(first.path()).unwrap());
    assert_ne!(owned, FileIdentity::from_path(second.path()).unwrap());
}

#[test]
fn retained_file_incarnation_survives_same_path_replacement_without_payload_copy() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("model.json");
    let previous = fixture.path().join("retained-model.json");
    std::fs::write(&path, PRIMARY).unwrap();
    let mut business_reader = File::open(&path).unwrap();
    let identity = FileIdentity::from_file(business_reader.try_clone().unwrap()).unwrap();
    std::fs::rename(&path, &previous).unwrap();
    std::fs::write(&path, PRIMARY).unwrap();
    assert_eq!(identity, FileIdentity::from_path(&previous).unwrap());
    assert_ne!(identity, FileIdentity::from_path(&path).unwrap());
    let mut actual = Vec::new();
    business_reader.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, PRIMARY);
}

#[cfg(unix)]
#[test]
fn retained_directory_incarnation_does_not_follow_a_replacement_label() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("workspace");
    let previous = fixture.path().join("retained-workspace");
    std::fs::create_dir(&path).unwrap();
    let directory = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
    let identity = FileIdentity::from_file(directory.try_clone().unwrap().into_std_file()).unwrap();
    std::fs::rename(&path, &previous).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert_eq!(identity, FileIdentity::from_path(&previous).unwrap());
    assert_ne!(identity, FileIdentity::from_path(&path).unwrap());
}

#[test]
fn missing_path_query_returns_an_error_without_touching_an_existing_reader() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("model.json");
    std::fs::write(&path, PRIMARY).unwrap();
    let mut reader = File::open(&path).unwrap();
    reader.seek(SeekFrom::Start(5)).unwrap();
    let failure = FileIdentity::from_path(&fixture.path().join("missing.json")).unwrap_err();
    assert_eq!(failure.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(reader.stream_position().unwrap(), 5);
    assert_eq!(std::fs::read(&path).unwrap(), PRIMARY);
}

#[test]
fn identity_query_keeps_opaque_private_record_bytes_out_of_a_model_decoder() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("opaque-record.json");
    std::fs::write(&path, PRIVATE).unwrap();
    let mut reader = File::open(&path).unwrap();
    let identity = FileIdentity::from_file(reader.try_clone().unwrap()).unwrap();
    assert_eq!(identity, FileIdentity::from_path(&path).unwrap());
    assert_eq!(reader.stream_position().unwrap(), 0);
    let mut actual = Vec::new();
    reader.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, PRIVATE);
}

#[test]
fn actual_owner_moves_through_a_joined_worker_without_a_clone_or_raw_identifier() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("model.json");
    std::fs::write(&path, PRIMARY).unwrap();
    let identity = FileIdentity::from_file(File::open(&path).unwrap()).unwrap();
    let worker_path = path.clone();
    let returned = std::thread::spawn(move || {
        assert_eq!(identity, FileIdentity::from_path(&worker_path).unwrap());
        identity
    })
    .join()
    .unwrap();
    assert_eq!(returned, FileIdentity::from_path(&path).unwrap());
}

#[test]
fn file_identity_is_not_a_document_revision_or_unchanged_content_proof() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("model.json");
    std::fs::write(&path, PRIMARY).unwrap();
    let identity = FileIdentity::from_file(File::open(&path).unwrap()).unwrap();
    std::fs::write(&path, PRIVATE).unwrap();
    assert_eq!(identity, FileIdentity::from_path(&path).unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), PRIVATE);
}

#[cfg(windows)]
#[test]
fn actual_pipe_identity_query_refuses_without_consuming_a_separate_business_reader() {
    use std::{io::Write as _, os::windows::io::OwnedHandle};
    let (reader, mut writer) = std::io::pipe().unwrap();
    let mut business_reader = reader.try_clone().unwrap();
    let file = File::from(OwnedHandle::from(reader));
    assert!(FileIdentity::from_file(file).is_err());
    writer.write_all(b"x").unwrap();
    let mut actual = [0_u8; 1];
    business_reader.read_exact(&mut actual).unwrap();
    assert_eq!(&actual, b"x");
}

#[cfg(unix)]
mod fifo_metadata {
    use super::FileIdentity;
    use std::{
        ffi::CString,
        fs::OpenOptions,
        os::unix::{ffi::OsStrExt as _, fs::OpenOptionsExt as _},
        path::PathBuf,
        process::{Child, Command},
        time::{Duration, Instant},
    };

    const CHILD_PATH: &str = "MORPHIR_FILE_IDENTITY_FIFO_QUERY_PATH";
    const CHILD_RESULT: &str = "MORPHIR_FILE_IDENTITY_FIFO_QUERY_RESULT";

    struct ChildOwner(Child);
    impl Drop for ChildOwner {
        fn drop(&mut self) {
            // The controller owns and reaps this genuine subprocess even when
            // the assertion fails. No FIFO writer is opened to rescue the SUT.
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn fifo_path_query_finishes_without_waiting_for_a_content_peer() {
        let fixture = tempfile::tempdir().unwrap();
        let fifo = fixture.path().join("query-only.fifo");
        let result = fixture.path().join("child-completed.txt");
        let native = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(native.as_ptr(), 0o600) }, 0);
        let mut child = ChildOwner(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "fifo_metadata::fifo_path_query_child",
                    "--nocapture",
                ])
                .env(CHILD_PATH, &fifo)
                .env(CHILD_RESULT, &result)
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                assert!(status.success(), "actual FIFO identity child: {status}");
                assert_eq!(std::fs::read(&result).unwrap(), b"queried-without-peer");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "FileIdentity::from_path waited for a FIFO content peer"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn fifo_path_query_child() {
        let Some(path) = std::env::var_os(CHILD_PATH) else {
            return; // Helper-only invocation, not an additional semantic case.
        };
        let path = PathBuf::from(path);
        let queried = FileIdentity::from_path(&path).unwrap();
        let actual = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
            .open(&path)
            .unwrap();
        assert_eq!(queried, FileIdentity::from_file(actual).unwrap());
        // Prove the exact child body ran rather than accepting a zero-test
        // subprocess. This is test completion, never a source-observer event.
        std::fs::write(
            PathBuf::from(std::env::var_os(CHILD_RESULT).unwrap()),
            b"queried-without-peer",
        )
        .unwrap();
    }
}

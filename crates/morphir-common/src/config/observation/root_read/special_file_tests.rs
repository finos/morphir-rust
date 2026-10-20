//! Real special-file acquisition, bounded by an owned OS test subprocess.
use super::*;
use std::{
    ffi::CString,
    os::unix::{ffi::OsStrExt, fs::FileTypeExt},
    process::{Child, Command},
    time::{Duration, Instant},
};
const CHILD_CASE: &str = "MORPHIR_ROOT_READ_SPECIAL_CASE";
const CHILD_ROOT: &str = "MORPHIR_ROOT_READ_SPECIAL_ROOT";
struct ChildOwner(Option<Child>);
impl Drop for ChildOwner {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
fn bounded_child(case: &str, test: &str, root: &Path) {
    let mut owner = ChildOwner(Some(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .env(CHILD_CASE, case)
            .env(CHILD_ROOT, root)
            .spawn()
            .unwrap(),
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = owner.0.as_mut().unwrap().try_wait().unwrap() {
            owner.0.take(); // try_wait actually reaped; no later PID kill.
            assert!(
                status.success(),
                "actual special-file child refused its semantic oracle: {status}"
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "actual {case} acquisition blocked beyond5s; owned child killed and reaped"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn make_fifo(path: &Path) {
    let path = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: terminated actual temporary fixture path; existing file creation only in test.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
}
fn observed_root(path: &Path) -> (Dir, SourceReadScope) {
    let root = Dir::open_ambient_dir(path, cap_std::ambient_authority()).unwrap();
    let scope = SourceReadScope::bind_workspace(
        &root,
        path,
        super::super::ReadLimits::new(8, 8, 1024, 0).unwrap(),
    )
    .unwrap();
    (root, scope)
}
fn zero_reads(scope: &SourceReadScope, attempts: u64) {
    let counts = scope.observation().snapshot().total_counts();
    assert_eq!(counts.content_attempts(), attempts);
    assert_eq!(counts.read_calls(), 0);
    assert_eq!(counts.returned_bytes(), 0);
}
#[test]
fn fifo_metadata_preparation_without_writer_refuses_without_content() {
    if std::env::var(CHILD_CASE).as_deref() == Ok("fifo-metadata") {
        let path = std::env::var_os(CHILD_ROOT).unwrap();
        let (root, scope) = observed_root(Path::new(&path));
        let file = RootRelativeFile::parse(Path::new("fifo")).unwrap();
        assert!(matches!(
            scope.prepare_root_read(&root, &RootRelativeDirectory::SelectedRoot, &file),
            Err(RootReadError::NonRegular)
        ));
        zero_reads(&scope, 0);
        assert_eq!(scope.seal().coverage(), super::super::Coverage::Unobserved);
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    make_fifo(&fixture.path().join("fifo"));
    bounded_child(
        "fifo-metadata",
        "config::observation::root_read::special_file_tests::fifo_metadata_preparation_without_writer_refuses_without_content",
        fixture.path(),
    );
}
#[test]
fn actual_null_device_metadata_is_refused_without_content() {
    assert!(
        std::fs::metadata("/dev/null")
            .unwrap()
            .file_type()
            .is_char_device()
    );
    if std::env::var(CHILD_CASE).as_deref() == Ok("null-metadata") {
        let (root, scope) = observed_root(Path::new("/dev"));
        let file = RootRelativeFile::parse(Path::new("null")).unwrap();
        assert!(matches!(
            scope.prepare_root_read(&root, &RootRelativeDirectory::SelectedRoot, &file),
            Err(RootReadError::NonRegular)
        ));
        zero_reads(&scope, 0);
        assert_eq!(scope.seal().coverage(), super::super::Coverage::Unobserved);
        return;
    }
    bounded_child(
        "null-metadata",
        "config::observation::root_read::special_file_tests::actual_null_device_metadata_is_refused_without_content",
        Path::new("/dev"),
    );
}
#[test]
fn fifo_replacing_pinned_regular_target_refuses_before_read() {
    if std::env::var(CHILD_CASE).as_deref() == Ok("fifo-data") {
        let path = std::env::var_os(CHILD_ROOT).unwrap();
        let path = Path::new(&path);
        let (root, scope) = observed_root(path);
        let file = RootRelativeFile::parse(Path::new("model.json")).unwrap();
        let mut plan = scope
            .prepare_root_read(&root, &RootRelativeDirectory::SelectedRoot, &file)
            .unwrap();
        let replace = || {
            std::fs::rename(path.join("model.json"), path.join("old-model.json")).unwrap();
            make_fifo(&path.join("model.json"));
        };
        plan.before_data_acquire = Some(&replace);
        assert!(matches!(
            scope.open_read_only_root(&plan),
            Err(RootReadError::NonRegular)
        ));
        zero_reads(&scope, 1);
        drop(plan);
        assert!(scope.seal().require_complete().is_err());
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    std::fs::write(
        fixture.path().join("model.json"),
        include_bytes!("../../../../tests/source_observation_fixtures/primary.json"),
    )
    .unwrap();
    bounded_child(
        "fifo-data",
        "config::observation::root_read::special_file_tests::fifo_replacing_pinned_regular_target_refuses_before_read",
        fixture.path(),
    );
}
#[test]
fn fifo_identity_registration_is_bounded_metadata_without_content() {
    if std::env::var(CHILD_CASE).as_deref() == Ok("fifo-register") {
        let path = std::env::var_os(CHILD_ROOT).unwrap();
        let root = Dir::open_ambient_dir(&path, cap_std::ambient_authority()).unwrap();
        // A positive witness scenario needs one caller-issued registry slot.
        let scope = SourceReadScope::bind_workspace(
            &root,
            Path::new(&path),
            super::super::ReadLimits::new(8, 8, 1024, 1).unwrap(),
        )
        .unwrap();
        let witness = scope.register_file(&root, Path::new("fifo")).unwrap();
        zero_reads(&scope, 0);
        let snapshot = scope.observation().snapshot();
        assert_eq!(
            snapshot.counts_for(&witness).unwrap(),
            super::super::ReadCounts::default()
        );
        assert_eq!(snapshot.coverage(), super::super::Coverage::Unobserved);
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    make_fifo(&fixture.path().join("fifo"));
    bounded_child(
        "fifo-register",
        "config::observation::root_read::special_file_tests::fifo_identity_registration_is_bounded_metadata_without_content",
        fixture.path(),
    );
}
#[test]
fn zero_file_budget_refuses_fifo_registration_promptly_without_content() {
    if std::env::var(CHILD_CASE).as_deref() == Ok("fifo-register-zero") {
        let path = std::env::var_os(CHILD_ROOT).unwrap();
        let (root, scope) = observed_root(Path::new(&path));
        assert!(matches!(
            scope.register_file(&root, Path::new("fifo")),
            Err(super::super::ObservationError::Capacity)
        ));
        zero_reads(&scope, 0);
        let snapshot = scope.observation().snapshot();
        assert_eq!(snapshot.coverage(), super::super::Coverage::Partial);
        assert!(snapshot.require_complete().is_err());
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    make_fifo(&fixture.path().join("fifo"));
    bounded_child(
        "fifo-register-zero",
        "config::observation::root_read::special_file_tests::zero_file_budget_refuses_fifo_registration_promptly_without_content",
        fixture.path(),
    );
}

//! Candidate-provider probes. Passing does not qualify a production provider.
#[path = "provider_support/filesystem.rs"]
mod filesystem;

#[path = "provider_support/ordinary_user.rs"]
mod ordinary_user;

#[cfg(windows)]
#[path = "provider_support/windows_write_through.rs"]
mod windows_write_through;

use std::fs;

fn probe_tempdir() -> std::io::Result<tempfile::TempDir> {
    let directory = tempfile::tempdir()?;
    #[cfg(windows)]
    ordinary_user::assert_if_requested(directory.path());
    Ok(directory)
}

#[cfg(windows)]
#[test]
#[ignore = "requires an explicitly provisioned standard-user CI account and private scratch directory"]
fn ordinary_user_identity_and_state_acl_are_observed() {
    ordinary_user::assert_executable_boundary();
    let root = probe_tempdir().unwrap();
    ordinary_user::assert_required(root.path());
    let path = root.path().join("state.sqlite");
    let store = state::Store::initialize(&path).unwrap();
    store.assert_settings();
    for name in ["state.sqlite", "state.sqlite-wal", "state.sqlite-shm"] {
        ordinary_user::assert_required(&root.path().join(name));
    }
}

#[test]
fn detects_the_actual_required_local_filesystem() {
    let root = probe_tempdir().unwrap();
    let environment = filesystem::environment(root.path()).unwrap();
    eprintln!("candidate environment: {environment:?}");
    assert_eq!(environment.os, std::env::consts::OS);
    assert_eq!(environment.architecture, std::env::consts::ARCH);
    assert!(
        environment.required_local_filesystem(),
        "unsupported candidate environment: {environment:?}"
    );
}

#[test]
fn promotion_never_overwrites_an_existing_winner() {
    let root = probe_tempdir().unwrap();
    let source = root.path().join("stage");
    let destination = root.path().join("winner");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("payload"), b"first").unwrap();
    filesystem::promote(&source, &destination).unwrap();
    assert!(!source.exists());
    fs::create_dir(&source).unwrap();
    fs::write(source.join("payload"), b"second").unwrap();
    assert!(filesystem::promote(&source, &destination).is_err());
    assert_eq!(fs::read(destination.join("payload")).unwrap(), b"first");
    assert_eq!(fs::read(source.join("payload")).unwrap(), b"second");
}

#[test]
fn rejects_static_hardlinks_before_reading() {
    let root = probe_tempdir().unwrap();
    let original = root.path().join("original");
    fs::write(&original, b"bytes").unwrap();
    assert!(filesystem::regular_single_link(&original).unwrap());
    fs::hard_link(&original, root.path().join("alias")).unwrap();
    assert!(!filesystem::regular_single_link(&original).unwrap());
}

#[cfg(unix)]
#[test]
fn rejects_symlinks_fifo_and_device_without_opening_them() {
    use std::{
        ffi::CString,
        os::unix::{ffi::OsStrExt, fs::symlink},
    };
    let root = probe_tempdir().unwrap();
    let link = root.path().join("link");
    symlink("missing", &link).unwrap();
    assert!(!filesystem::regular_single_link(&link).unwrap());
    let fifo = root.path().join("fifo");
    let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: NUL-terminated private test path and standard owner-only permissions.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(!filesystem::regular_single_link(&fifo).unwrap());
    assert!(!filesystem::regular_single_link(std::path::Path::new("/dev/null")).unwrap());
}

#[cfg(windows)]
#[test]
fn rejects_directory_junction_without_following_it() {
    let root = probe_tempdir().unwrap();
    let target = root.path().join("target");
    let link = root.path().join("junction");
    fs::create_dir(&target).unwrap();
    let status = std::process::Command::new("cmd")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "required normal-user junction setup failed"
    );
    assert!(!filesystem::regular_single_link(&link).unwrap());
}

#[path = "provider_support/process.rs"]
mod process;
#[path = "provider_support/state.rs"]
mod state;

#[test]
fn held_lock_excludes_a_peer_and_releases_after_real_child_death() {
    let root = probe_tempdir().unwrap();
    let mut child = process::ChildProbe::start("lock", root.path());
    let lock = process::lock_file(root.path()).unwrap();
    let contention = fs2::FileExt::try_lock_exclusive(&lock).unwrap_err();
    assert_eq!(
        contention.raw_os_error(),
        fs2::lock_contended_error().raw_os_error()
    );
    child.kill_and_wait();
    fs2::FileExt::try_lock_exclusive(&lock).unwrap();
    assert!(root.path().join("repository.lock").exists());
}

#[test]
fn state_requires_explicit_complete_non_overwriting_initialization() {
    let root = probe_tempdir().unwrap();
    let path = root.path().join("state.sqlite");
    assert!(state::Store::open(&path).is_err());
    assert!(!path.exists());
    fs::write(&path, []).unwrap();
    assert!(state::Store::open(&path).is_err());
    assert!(state::Store::initialize(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"");
    fs::remove_file(&path).unwrap();
    let store = state::Store::initialize(&path).unwrap();
    assert_eq!(store.snapshot().unwrap(), (0, 0));
    assert!(state::Store::initialize(&path).is_err());
    drop(store);
    state::Store::open(&path).unwrap().assert_settings();
    let corrupt = root.path().join("corrupt.sqlite");
    fs::write(&corrupt, b"not sqlite").unwrap();
    assert!(state::Store::open(&corrupt).is_err());
    assert!(state::Store::initialize(&corrupt).is_err());
    assert_eq!(fs::read(&corrupt).unwrap(), b"not sqlite");
}

#[test]
fn killed_transactions_keep_marker_and_grant_atomic_across_fresh_processes() {
    for phase in ["before-commit", "after-commit"] {
        let root = probe_tempdir().unwrap();
        drop(state::Store::initialize(&root.path().join("state.sqlite")).unwrap());
        let mut child = process::ChildProbe::start(phase, root.path());
        child.kill_and_wait();
        let mut reader = process::ChildProbe::start(
            if phase == "before-commit" {
                "read-marker"
            } else {
                "read-grant"
            },
            root.path(),
        );
        reader.expect_success();
    }
}

// The parent selects this exact test in a separate executable process. Outside a child,
// it returns without inventing qualification evidence for an unexecuted operation.
#[test]
fn provider_child() {
    let Ok(mode) = std::env::var("MORPHIR_PROVIDER_CHILD") else {
        return;
    };
    let root = std::path::PathBuf::from(std::env::var_os("MORPHIR_PROVIDER_ROOT").unwrap());
    #[cfg(windows)]
    ordinary_user::assert_if_requested(&root);
    process::run_child(&mode, &root);
}

#[cfg(unix)]
#[test]
fn native_file_and_directory_flushes_succeed() {
    let root = probe_tempdir().unwrap();
    let path = root.path().join("file");
    fs::write(&path, b"persist these bytes").unwrap();
    filesystem::flush(&fs::OpenOptions::new().write(true).open(path).unwrap()).unwrap();
    filesystem::flush(&fs::File::open(root.path()).unwrap()).unwrap();
}

#[cfg(unix)]
#[test]
fn native_flush_errors_propagate() {
    use std::os::fd::OwnedFd;
    let (socket, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
    let fd: OwnedFd = socket.into();
    let file = fs::File::from(fd);
    assert!(filesystem::flush(&file).is_err());
}

#[test]
fn sqlite_rejects_incomplete_provisioning_schema() {
    let root = probe_tempdir().unwrap();
    let path = root.path().join("state.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TABLE provisioning(id INTEGER PRIMARY KEY, complete INTEGER, identity BLOB); INSERT INTO provisioning VALUES(1,0,X'00');").unwrap();
    drop(connection);
    assert!(state::Store::open(&path).is_err());
    assert!(state::Store::initialize(&path).is_err());
}

#[test]
fn failed_transaction_cannot_publish_a_grant() {
    let root = probe_tempdir().unwrap();
    let path = root.path().join("state.sqlite");
    let mut store = state::Store::initialize(&path).unwrap();
    store.commit_marker().unwrap();
    state::fail_grant_transaction(&mut store);
    drop(store);
    let mut reader = process::ChildProbe::start("read-marker", root.path());
    reader.expect_success();
}

#[test]
fn sqlite_full_write_failure_keeps_the_committed_marker() {
    let root = probe_tempdir().unwrap();
    let mut store = state::Store::initialize(&root.path().join("state.sqlite")).unwrap();
    store.commit_marker().unwrap();
    state::fail_full_write(&mut store);
    drop(store);
    let mut reader = process::ChildProbe::start("read-marker", root.path());
    reader.expect_success();
}

#[test]
fn promotion_refuses_even_an_empty_existing_directory() {
    let root = probe_tempdir().unwrap();
    let source = root.path().join("stage");
    let destination = root.path().join("empty-winner");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("payload"), b"candidate").unwrap();
    fs::create_dir(&destination).unwrap();
    assert!(filesystem::promote(&source, &destination).is_err());
    assert_eq!(fs::read(source.join("payload")).unwrap(), b"candidate");
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
}

#[cfg(windows)]
#[test]
fn windows_write_through_preserves_empty_directories_and_ancestors() {
    let root = probe_tempdir().unwrap();
    windows_write_through::stage_tree(root.path());
    windows_write_through::assert_tree(root.path(), "ancestors/stage");
    windows_write_through::promote_tree(root.path()).unwrap();
    assert!(!root.path().join("ancestors/stage").exists());
    windows_write_through::assert_tree(root.path(), "ancestors/winner");
}

#[cfg(windows)]
#[test]
fn windows_write_through_collision_preserves_both_trees() {
    for occupied in [false, true] {
        let root = probe_tempdir().unwrap();
        windows_write_through::stage_tree(root.path());
        let winner = root.path().join("ancestors/winner");
        fs::create_dir(&winner).unwrap();
        if occupied {
            fs::write(winner.join("previous"), b"existing winner").unwrap();
        }
        let error = windows_write_through::promote_tree(root.path()).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        windows_write_through::assert_tree(root.path(), "ancestors/stage");
        assert_eq!(
            fs::read_dir(&winner).unwrap().count(),
            usize::from(occupied)
        );
        if occupied {
            assert_eq!(
                fs::read(winner.join("previous")).unwrap(),
                b"existing winner"
            );
        }
    }
}

#[cfg(windows)]
#[test]
fn windows_write_through_tree_survives_writer_death_and_fresh_reader() {
    for phase in ["windows-staged", "windows-promoted"] {
        let root = probe_tempdir().unwrap();
        let mut child = process::ChildProbe::start(phase, root.path());
        child.kill_and_wait();
        let mut reader = process::ChildProbe::start(
            if phase == "windows-staged" {
                "windows-read-stage"
            } else {
                "windows-read-winner"
            },
            root.path(),
        );
        reader.expect_success();
    }
}

#[path = "provider_support/durability.rs"]
mod durability;
#[path = "provider_support/tree_durability.rs"]
mod tree_durability;
#[path = "provider_support/vfs_fault.rs"]
mod vfs_fault;

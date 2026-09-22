//! Fault and restart probes for the native SQLite implementation.
use super::{probe_tempdir, process, state};

use super::vfs_fault::{File, Intercept, Operation};
use std::path::Path;
const GRANT: &str = "BEGIN IMMEDIATE; INSERT INTO grant_record SELECT id,evidence FROM marker; DELETE FROM marker; COMMIT;";

fn marked_store(root: &Path) {
    let mut store = state::Store::initialize(&root.join("state.sqlite")).unwrap();
    store.commit_marker().unwrap();
}

fn operation_counts(target: File) -> super::vfs_fault::Counts {
    let root = probe_tempdir().unwrap();
    marked_store(root.path());
    let mut store = state::Store::open(&root.path().join("state.sqlite")).unwrap();
    assert_eq!(store.snapshot().unwrap(), (1, 0));
    let mut observer = Intercept::install(&mut store.connection, target, None);
    match target {
        File::Wal => observer.connection().execute_batch(GRANT).unwrap(),
        File::Database => {
            observer.connection().execute_batch(GRANT).unwrap();
            observer
                .connection()
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .unwrap();
        }
    }
    let counts = observer.counts();
    assert!(
        counts.writes > 0 && counts.syncs > 0,
        "native positive control must exercise both operations: {counts:?}"
    );
    assert_eq!(counts.failures, 0);
    drop(observer);
    drop(store);
    let mut reader = process::ChildProbe::start("read-grant", root.path());
    reader.expect_success();
    counts
}

#[test]
fn native_wal_faults_return_errors_and_recover_atomic_state() {
    let counts = operation_counts(File::Wal);
    for (kind, count) in [("write", counts.writes), ("sync", counts.syncs)] {
        for at in 1..=count {
            let root = probe_tempdir().unwrap();
            marked_store(root.path());
            let mut writer = process::ChildProbe::start(&format!("wal-{kind}-{at}"), root.path());
            writer.kill_and_wait();
            let mut reader = process::ChildProbe::start(
                if kind == "sync" {
                    "read-atomic"
                } else {
                    "read-marker"
                },
                root.path(),
            );
            reader.expect_success();
        }
    }
}

#[test]
fn native_checkpoint_write_and_sync_faults_keep_committed_grant() {
    let counts = operation_counts(File::Database);
    for (kind, count) in [("write", counts.writes), ("sync", counts.syncs)] {
        for at in 1..=count {
            let root = probe_tempdir().unwrap();
            marked_store(root.path());
            let mut writer =
                process::ChildProbe::start(&format!("checkpoint-{kind}-{at}"), root.path());
            writer.kill_and_wait();
            let mut reader = process::ChildProbe::start("read-grant", root.path());
            reader.expect_success();
        }
    }
}

pub fn fail_native_operation(root: &Path, mode: &str) -> ! {
    let parts: Vec<_> = mode.split('-').collect();
    let target = match parts[0] {
        "wal" => File::Wal,
        "checkpoint" => File::Database,
        _ => panic!("invalid fault target"),
    };
    let operation = match parts[1] {
        "write" => Operation::Write,
        "sync" => Operation::Sync,
        _ => panic!("invalid fault operation"),
    };
    let at = parts[2].parse::<usize>().unwrap();
    let mut store = state::Store::open(&root.join("state.sqlite")).unwrap();
    assert_eq!(store.snapshot().unwrap(), (1, 0));
    if matches!(target, File::Database) {
        store.connection.execute_batch(GRANT).unwrap();
    }
    let mut fault = Intercept::install(&mut store.connection, target, Some((operation, at)));
    let failure = fault
        .connection()
        .execute_batch(match target {
            File::Wal => GRANT,
            File::Database => "PRAGMA wal_checkpoint(TRUNCATE)",
        })
        .unwrap_err();
    assert_eq!(
        failure.sqlite_error().unwrap().extended_code,
        match operation {
            Operation::Write => rusqlite::ffi::SQLITE_IOERR_WRITE,
            Operation::Sync => rusqlite::ffi::SQLITE_IOERR_FSYNC,
        }
    );
    assert!(
        fault.counts().failures > 0,
        "fault must intercept a real native VFS call"
    );
    eprintln!("native {mode}: {:?}, {failure}", fault.counts());
    // Connection remains live until actual process death; no close/checkpoint cleanup.
    super::process::hold();
}

#[test]
fn initialization_death_before_completion_stays_closed_in_a_fresh_process() {
    for phase in ["init-reserved", "init-before-commit", "init-after-commit"] {
        let root = probe_tempdir().unwrap();
        let mut writer = process::ChildProbe::start(phase, root.path());
        writer.kill_and_wait();
        let mut reader = process::ChildProbe::start(
            if phase == "init-after-commit" {
                "read-initialized"
            } else {
                "read-uninitialized"
            },
            root.path(),
        );
        reader.expect_success();
    }
}

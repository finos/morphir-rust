//! Independent actual descriptor-acquisition boundary, frozen before repair.
//! Unit child of config::observation, so the phase hook stays private cfg(test).
use super::*;
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _, OpenOptionsSyncExt as _};
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(unix)]
#[test]
fn moved_out_subdirectory_data_open_cannot_become_complete_workspace_after_move_back() {
    // Workspace and outside sibling share one genuine filesystem parent. These
    // are actual directory renames, not labels, fake readers or event setters.
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("workspace");
    let child = workspace.join("child");
    let outside = fixture.path().join("outside-child");
    std::fs::create_dir_all(&child).unwrap();
    let expected = include_bytes!("../../../tests/source_observation_fixtures/primary.json");
    assert_eq!(expected.len(), 111);
    std::fs::write(child.join("model.json"), expected).unwrap();
    let root = Dir::open_ambient_dir(&workspace, cap_std::ambient_authority()).unwrap();
    let held_child = root.open_dir("child").unwrap();
    let scope = SourceReadScope::bind_workspace(
        &root,
        &workspace,
        ReadLimits::new(64, 1024, 16_777_216, 16).unwrap(),
    )
    .unwrap();
    let actual_file = scope
        .register_file(&held_child, Path::new("model.json"))
        .unwrap();
    let hook_calls = AtomicUsize::new(0);
    let move_back = || {
        // The hook is invoked ONLY after the actual data File was acquired.
        // It cannot create/replace the reader, choose results or write counters.
        assert_eq!(hook_calls.fetch_add(1, Ordering::SeqCst), 0);
        assert!(outside.is_dir());
        assert!(!child.exists());
        std::fs::rename(&outside, &child).unwrap();
    };
    let mut directory = scope.content_directory(&held_child, Some(&child));
    directory.after_data_open = Some(&move_back);
    // Cached provenance was obtained inside, but the actual data open below
    // occurs through this SAME held child after it is outside selected root.
    std::fs::rename(&child, &outside).unwrap();
    assert!(outside.join("model.json").is_file());
    assert!(!child.exists());
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No).nonblock(true);
    let mut file = directory
        .open_content_file(Path::new("model.json"), &options)
        .expect("observation must preserve the caller-held grant's actual business open");
    assert_eq!(hook_calls.load(Ordering::SeqCst), 1);
    assert!(child.is_dir());
    assert!(!outside.exists());
    assert_eq!(file.metadata().unwrap().len(), 111);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, expected);
    drop(file);
    drop(directory);
    let snapshot = scope.seal();
    let actual = snapshot.counts_for(&actual_file).unwrap();
    assert_eq!(actual.content_attempts(), 1);
    assert!(actual.read_calls() >= 1);
    assert_eq!(actual.returned_bytes(), 111);
    assert!(snapshot.total_counts().content_attempts() >= 1);
    assert_eq!(snapshot.total_counts().returned_bytes(), 111);
    // A sound implementation may conservatively retain Partial when it cannot
    // prove acquisition, or report a known actual Ambient route. Unobserved or
    // Complete WorkspaceContent cannot represent this observed outside open.
    match snapshot.coverage() {
        Coverage::Partial => assert!(snapshot.require_complete().is_err()),
        Coverage::Complete => {
            snapshot.require_complete().unwrap();
            assert_eq!(
                snapshot
                    .counts(ReadClass::WorkspaceContent)
                    .content_attempts(),
                0,
                "post-open inode equality does not turn the earlier outside data open into a rooted acquisition"
            );
            assert_eq!(
                snapshot
                    .counts(ReadClass::WorkspaceContent)
                    .returned_bytes(),
                0
            );
            assert_eq!(
                snapshot
                    .counts(ReadClass::AmbientContent)
                    .content_attempts(),
                1
            );
            assert_eq!(
                snapshot.counts(ReadClass::AmbientContent).returned_bytes(),
                111
            );
        }
        Coverage::Unobserved => panic!("the actual successful open/read was observed"),
    }
}

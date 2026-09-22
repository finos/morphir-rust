//! Completed-call ordering probes on private fixture trees; not a production provider.
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Step {
    CreateDirectory(&'static str),
    WriteFile(&'static str),
    FlushFile(&'static str),
    #[cfg(unix)]
    FlushDirectory(&'static str),
    Promote,
}

fn publish(root: &Path, mut before: impl FnMut(Step) -> io::Result<()>) -> io::Result<()> {
    for name in [
        "ancestors",
        "ancestors/stage",
        "ancestors/stage/empty",
        "ancestors/stage/nested",
        "ancestors/stage/nested/empty-leaf",
    ] {
        before(Step::CreateDirectory(name))?;
        #[cfg(unix)]
        fs::create_dir(root.join(name))?;
        #[cfg(windows)]
        super::windows_write_through::create_directory(&root.join(name))?;
    }
    for (name, bytes) in [
        (
            "ancestors/stage/nested/payload",
            b"candidate payload".as_slice(),
        ),
        ("ancestors/stage/nested/empty-file", b"".as_slice()),
    ] {
        before(Step::WriteFile(name))?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_WRITE_THROUGH);
        }
        let mut file = options.open(root.join(name))?;
        file.write_all(bytes)?;
        before(Step::FlushFile(name))?;
        #[cfg(unix)]
        super::filesystem::flush(&file)?;
        #[cfg(windows)]
        file.sync_all()?;
    }
    // Leaves before parents persist empty entries and the staged root's ancestors.
    // Windows creation itself uses synchronous native write-through handles.
    #[cfg(unix)]
    for name in [
        "ancestors/stage/empty",
        "ancestors/stage/nested/empty-leaf",
        "ancestors/stage/nested",
        "ancestors/stage",
        "ancestors",
        "",
    ] {
        before(Step::FlushDirectory(name))?;
        super::filesystem::flush(&fs::File::open(root.join(name))?)?;
    }
    before(Step::Promote)?;
    #[cfg(unix)]
    super::filesystem::promote(
        &root.join("ancestors/stage"),
        &root.join("ancestors/winner"),
    )?;
    #[cfg(windows)]
    super::windows_write_through::promote_tree(root)?;
    #[cfg(unix)]
    {
        before(Step::FlushDirectory("ancestors"))?;
        super::filesystem::flush(&fs::File::open(root.join("ancestors"))?)?;
    }
    Ok(())
}
fn assert_tree(root: &Path, name: &str) {
    let tree = root.join(name);
    assert_eq!(
        fs::read(tree.join("nested/payload")).unwrap(),
        b"candidate payload"
    );
    assert_eq!(fs::read(tree.join("nested/empty-file")).unwrap(), b"");
    for empty in ["empty", "nested/empty-leaf"] {
        assert_eq!(fs::read_dir(tree.join(empty)).unwrap().count(), 0);
    }
}
#[test]
fn whole_tree_is_flushed_before_promotion_and_parent_afterward() {
    let root = super::probe_tempdir().unwrap();
    let mut steps = Vec::new();
    publish(root.path(), |step| {
        steps.push(step);
        Ok(())
    })
    .unwrap();
    assert_tree(root.path(), "ancestors/winner");
    assert!(!root.path().join("ancestors/stage").exists());
    let promotion = steps
        .iter()
        .position(|step| *step == Step::Promote)
        .unwrap();
    for file in [
        "ancestors/stage/nested/payload",
        "ancestors/stage/nested/empty-file",
    ] {
        assert!(
            steps
                .iter()
                .position(|step| *step == Step::FlushFile(file))
                .unwrap()
                < promotion
        );
    }
    #[cfg(unix)]
    {
        let before_promotion = &steps[..promotion];
        let assert_before = |earlier: Step, later: Step| {
            let earlier_index = before_promotion
                .iter()
                .position(|step| *step == earlier)
                .unwrap_or_else(|| panic!("missing operation before promotion: {earlier:?}"));
            let later_index = before_promotion
                .iter()
                .position(|step| *step == later)
                .unwrap_or_else(|| panic!("missing operation before promotion: {later:?}"));
            assert!(
                earlier_index < later_index,
                "persistence dependency violated: {earlier:?} must precede {later:?}"
            );
        };
        for file in [
            "ancestors/stage/nested/payload",
            "ancestors/stage/nested/empty-file",
        ] {
            assert_before(
                Step::FlushFile(file),
                Step::FlushDirectory("ancestors/stage/nested"),
            );
        }
        // Assert only containment dependencies, leaving sibling order unconstrained.
        for (child, parent) in [
            (
                "ancestors/stage/nested/empty-leaf",
                "ancestors/stage/nested",
            ),
            ("ancestors/stage/empty", "ancestors/stage"),
            ("ancestors/stage/nested", "ancestors/stage"),
            ("ancestors/stage", "ancestors"),
            ("ancestors", ""),
        ] {
            assert_before(Step::FlushDirectory(child), Step::FlushDirectory(parent));
        }
        assert_eq!(
            &steps[promotion + 1..],
            &[Step::FlushDirectory("ancestors")]
        );
    }
}

#[test]
fn each_failed_tree_operation_stops_publication_without_fallback() {
    let baseline = super::probe_tempdir().unwrap();
    let mut steps = Vec::new();
    publish(baseline.path(), |step| {
        steps.push(step);
        Ok(())
    })
    .unwrap();
    assert!(!steps.is_empty());
    let promotion = steps
        .iter()
        .position(|step| *step == Step::Promote)
        .unwrap();
    for failed in 0..steps.len() {
        let root = super::probe_tempdir().unwrap();
        let mut visited = Vec::new();
        let error = publish(root.path(), |step| {
            visited.push(step);
            if visited.len() == failed + 1 {
                Err(io::Error::other("injected native boundary failure"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "injected native boundary failure");
        assert_eq!(visited, steps[..=failed]);
        if failed <= promotion {
            assert!(!root.path().join("ancestors/winner").exists());
        } else {
            // Rename completed, then directory synchronization failed. Returning
            // failure must not imply rollback or allow a grant from row/tree presence.
            assert_tree(root.path(), "ancestors/winner");
            assert!(!root.path().join("ancestors/stage").exists());
        }
    }
}

#[test]
fn native_collision_preserves_the_staged_tree_and_existing_winner() {
    let root = super::probe_tempdir().unwrap();
    let error = publish(root.path(), |step| {
        if step == Step::Promote {
            fs::create_dir(root.path().join("ancestors/winner"))?;
            fs::write(
                root.path().join("ancestors/winner/previous"),
                b"original winner",
            )?;
        }
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_tree(root.path(), "ancestors/stage");
    assert_eq!(
        fs::read(root.path().join("ancestors/winner/previous")).unwrap(),
        b"original winner"
    );
    assert_eq!(
        fs::read_dir(root.path().join("ancestors/winner"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn staged_and_promoted_trees_survive_real_writer_death_and_fresh_reader() {
    for name in ["stage", "winner"] {
        let root = super::probe_tempdir().unwrap();
        let mut writer = super::process::ChildProbe::start(&format!("tree-{name}"), root.path());
        writer.kill_and_wait();
        let mut reader =
            super::process::ChildProbe::start(&format!("tree-read-{name}"), root.path());
        reader.expect_success();
    }
}

pub fn run_child(mode: &str, root: &Path) {
    match mode {
        "tree-stage" | "tree-winner" => {
            publish(root, |step| {
                if mode == "tree-stage" && step == Step::Promote {
                    super::process::hold();
                }
                Ok(())
            })
            .unwrap();
            super::process::hold();
        }
        "tree-read-stage" | "tree-read-winner" => {
            let promoted = mode == "tree-read-winner";
            assert_tree(
                root,
                if promoted {
                    "ancestors/winner"
                } else {
                    "ancestors/stage"
                },
            );
            assert!(
                !root
                    .join(if promoted {
                        "ancestors/stage"
                    } else {
                        "ancestors/winner"
                    })
                    .exists()
            );
            super::process::ready();
        }
        _ => panic!("unknown tree child operation"),
    }
}

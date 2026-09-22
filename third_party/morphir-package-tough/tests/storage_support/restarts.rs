use super::*;
fn child(store: &Sqlite, fixture: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "restarts::storage_child", "--nocapture"])
        .env("MORPHIR_TUF_PROBE_DB", &store.path)
        .env("MORPHIR_TUF_PROBE_FIXTURE", fixture)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    command
}
#[tokio::test]
async fn storage_child() {
    let Some(path) = std::env::var_os("MORPHIR_TUF_PROBE_DB") else {
        return;
    };
    let fixture = std::env::var_os("MORPHIR_TUF_PROBE_FIXTURE").unwrap();
    let store = Arc::new(Sqlite { path: path.into() });
    let result = load(
        store,
        Path::new(&fixture),
        ProbeAdmission::permit_for_storage_probe_only(),
        "2026-01-01T00:00:00Z",
    )
    .await;
    if std::env::var_os("MORPHIR_TUF_PROBE_EXPECT_ERROR").is_some() {
        assert!(result.is_err());
    } else {
        result.unwrap();
    }
}

#[tokio::test]
async fn process_kill_exposes_atomic_root_and_reset_context_then_restart_finishes_reset() {
    for (phase, point) in [
        ("root", "before"),
        ("root", "after"),
        ("finish", "before"),
        ("finish", "after"),
    ] {
        let (directory, store, fixture) = initialized();
        let (timestamp, snapshot, _) = fixtures::view(50, 17);
        seed_floor(&store, MetadataRole::Timestamp, &timestamp);
        seed_floor(&store, MetadataRole::Snapshot, &snapshot);
        let signal = directory.path().join("ready");
        let mut process = child(&store, &fixture)
            .env("MORPHIR_TUF_PROBE_BARRIER", format!("{phase}:{point}"))
            .env("MORPHIR_TUF_PROBE_SIGNAL", &signal)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !signal.exists() {
            assert!(
                process.try_wait().unwrap().is_none(),
                "child ended before transaction barrier"
            );
            if Instant::now() >= deadline {
                process.kill().unwrap();
                process.wait().unwrap();
                panic!("transaction barrier timed out");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        process.kill().unwrap();
        process.wait().unwrap();
        let state = store.snapshot().await.unwrap();
        if phase == "root" && point == "before" {
            assert_eq!(state.current_root, fixtures::root(1, 17));
            assert!(state.reset_baseline.is_none());
        } else {
            assert_eq!(state.current_root, fixtures::root(2, 18));
            if phase == "finish" && point == "after" {
                assert!(state.reset_baseline.is_none());
            } else {
                assert_eq!(state.reset_baseline, Some(fixtures::root(1, 17)));
            }
            fs::remove_file(fixture.join("2.root.json")).unwrap();
        }
        if phase == "finish" && point == "after" {
            assert!(!state.metadata.contains_key(&MetadataRole::Timestamp));
            assert!(!state.metadata.contains_key(&MetadataRole::Snapshot));
        } else {
            assert_eq!(state.metadata[&MetadataRole::Timestamp].bytes, timestamp);
            assert_eq!(state.metadata[&MetadataRole::Snapshot].bytes, snapshot);
        }
        assert!(child(&store, &fixture).status().unwrap().success());
        let recovered = store.snapshot().await.unwrap();
        assert_eq!(recovered.current_root, fixtures::root(2, 18));
        assert!(recovered.reset_baseline.is_none());
        let (timestamp, snapshot, _) = fixtures::view(1, 18);
        assert_eq!(
            recovered.metadata[&MetadataRole::Timestamp].bytes,
            timestamp
        );
        assert_eq!(recovered.metadata[&MetadataRole::Snapshot].bytes, snapshot);
    }
}

#[tokio::test]
async fn failed_root_or_reset_commit_does_not_publish_partial_success() {
    for phase in ["root", "finish"] {
        let (_directory, store, fixture) = initialized();
        let (timestamp, snapshot, _) = fixtures::view(50, 17);
        seed_floor(&store, MetadataRole::Timestamp, &timestamp);
        seed_floor(&store, MetadataRole::Snapshot, &snapshot);
        assert!(child(&store, &fixture)
            .env("MORPHIR_TUF_PROBE_FAIL", phase)
            .env("MORPHIR_TUF_PROBE_EXPECT_ERROR", "1")
            .status()
            .unwrap()
            .success());
        let state = store.snapshot().await.unwrap();
        assert_eq!(state.metadata[&MetadataRole::Timestamp].bytes, timestamp);
        assert_eq!(state.metadata[&MetadataRole::Snapshot].bytes, snapshot);
        if phase == "root" {
            assert_eq!(state.revision, Revision(0));
            assert_eq!(state.current_root, fixtures::root(1, 17));
            assert!(state.reset_baseline.is_none());
        } else {
            assert_eq!(state.current_root, fixtures::root(2, 18));
            assert_eq!(state.reset_baseline, Some(fixtures::root(1, 17)));
        }
        assert!(child(&store, &fixture).status().unwrap().success());
    }
}

#[tokio::test]
async fn cas_child() {
    let Some(path) = std::env::var_os("MORPHIR_TUF_PROBE_DB") else {
        return;
    };
    let store = Sqlite { path: path.into() };
    let transition = Transition::AdvanceRoot {
        root: fixtures::root(2, 18),
        baseline: fixtures::root(1, 17),
    };
    let outcome = match store.commit(Revision(0), &transition).await {
        Ok(_) => "committed",
        Err(StoreError::Conflict) => "conflict",
        Err(error) => panic!("{}", error),
    };
    fs::write(
        std::env::var_os("MORPHIR_TUF_PROBE_OUTCOME").unwrap(),
        outcome,
    )
    .unwrap();
}
#[tokio::test]
async fn separate_processes_cannot_commit_the_same_predecessor_twice() {
    let (directory, store, _fixture) = initialized();
    let mut children = Vec::new();
    for index in 0..2 {
        children.push(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "restarts::cas_child"])
                .env("MORPHIR_TUF_PROBE_DB", &store.path)
                .env(
                    "MORPHIR_TUF_PROBE_OUTCOME",
                    directory.path().join(index.to_string()),
                )
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
    }
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let mut results = (0..2)
        .map(|index| fs::read_to_string(directory.path().join(index.to_string())).unwrap())
        .collect::<Vec<_>>();
    results.sort();
    assert_eq!(results, vec!["committed", "conflict"]);
    let state = store.snapshot().await.unwrap();
    assert_eq!(state.revision, Revision(1));
    assert_eq!(state.current_root, fixtures::root(2, 18));
}

#[tokio::test]
async fn threshold_update_restarts_using_each_roles_retained_acceptance_root() {
    for failure_phase in ["finish", "metadata"] {
        let directory = TempDir::new().unwrap();
        let fixture = directory.path().join("fixture");
        fs::create_dir(&fixture).unwrap();
        let root = fixtures::threshold_root(1, 1);
        fs::write(fixture.join("1.root.json"), &root).unwrap();
        fixtures::write_threshold_view(&fixture, 10, &[18]);
        let store = Arc::new(Sqlite::initialize(
            &directory.path().join("state.db"),
            &root,
        ));
        load(
            store.clone(),
            &fixture,
            ProbeAdmission::permit_for_storage_probe_only(),
            "2026-01-01T00:00:00Z",
        )
        .await
        .unwrap();
        fs::write(fixture.join("2.root.json"), fixtures::threshold_root(2, 2)).unwrap();
        fixtures::write_threshold_view(&fixture, 11, &[18, 19]);
        // Fail after root advancement, either before finishing its cycle or while
        // replacing role evidence. Both exact old bytes and authority must survive.
        let before = store.snapshot().await.unwrap();
        assert!(child(&store, &fixture)
            .env("MORPHIR_TUF_PROBE_FAIL", failure_phase)
            .env("MORPHIR_TUF_PROBE_EXPECT_ERROR", "1")
            .status()
            .unwrap()
            .success());
        let interrupted = store.snapshot().await.unwrap();
        assert_eq!(interrupted.current_root, fixtures::threshold_root(2, 2));
        for role in [MetadataRole::Timestamp, MetadataRole::Snapshot] {
            assert_eq!(interrupted.metadata[&role].acceptance_root, root);
            assert_eq!(
                interrupted.metadata[&role].bytes,
                before.metadata[&role].bytes
            );
        }
        // Resumption must use the retained current root, not redownload its body.
        fs::remove_file(fixture.join("2.root.json")).unwrap();
        assert!(child(&store, &fixture).status().unwrap().success());
        let recovered = store.snapshot().await.unwrap();
        assert!(recovered.reset_baseline.is_none());
        for role in [MetadataRole::Timestamp, MetadataRole::Snapshot] {
            assert_eq!(
                recovered.metadata[&role].acceptance_root,
                fixtures::threshold_root(2, 2)
            );
            let document: serde_json::Value =
                serde_json::from_slice(&recovered.metadata[&role].bytes).unwrap();
            assert_eq!(document["signed"]["version"], 11);
        }
    }
}

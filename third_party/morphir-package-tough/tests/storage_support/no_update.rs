//! Equality stops current metadata loading; it does not establish a fresh complete view.
use super::*;
use tough::experimental_storage::PackageLoadOutcome;

fn initial_view() -> (TempDir, Arc<Sqlite>, std::path::PathBuf) {
    let directory = TempDir::new().unwrap();
    let fixture = directory.path().join("fixture");
    fs::create_dir(&fixture).unwrap();
    let root = fixtures::root(1, 18);
    fs::write(fixture.join("1.root.json"), &root).unwrap();
    fixtures::write_view(&fixture, 10);
    let store = Arc::new(Sqlite::initialize(
        &directory.path().join("state.db"),
        &root,
    ));
    (directory, store, fixture)
}
async fn update(
    store: Arc<Sqlite>,
    fixture: &Path,
) -> Result<PackageLoadOutcome, Box<tough::error::Error>> {
    RepositoryLoader::new(
        &fs::read(fixture.join("1.root.json")).unwrap(),
        test_utils::dir_url(fixture),
        test_utils::dir_url(fixture),
    )
    .fixed_time("2026-01-01T00:00:00Z".parse().unwrap())
    .experimental_storage(
        store,
        Arc::new(ProbeAdmission::permit_for_storage_probe_only()),
    )
    .load_package()
    .await
    .map_err(Box::new)
}
fn candidate(
    fixture: &Path,
    timestamp_version: u64,
    snapshot_version: u64,
    expires: &str,
    seed: u8,
) -> Vec<u8> {
    let (_, snapshot, targets) = fixtures::view(snapshot_version, 18);
    let timestamp = fixtures::timestamp(
        timestamp_version,
        snapshot_version,
        &snapshot,
        expires,
        seed,
    );
    fs::write(fixture.join("timestamp.json"), &timestamp).unwrap();
    fs::write(
        fixture.join(format!("{snapshot_version}.snapshot.json")),
        snapshot,
    )
    .unwrap();
    fs::write(
        fixture.join(format!("{snapshot_version}.targets.json")),
        targets,
    )
    .unwrap();
    timestamp
}
const FRESH: &str = "2100-01-01T00:00:00Z";
const EXPIRED: &str = "2000-01-01T00:00:00Z";

#[tokio::test]
async fn equality_discards_fresh_or_expired_changed_timestamp_before_snapshot_checks_or_writes() {
    for expires in [FRESH, EXPIRED] {
        // Lower link proves equality precedes snapshot rollback comparison; higher
        // link with no file proves equality does not fetch a replacement snapshot.
        for next_snapshot in [9, 11] {
            let (_directory, store, fixture) = initial_view();
            assert!(matches!(
                update(store.clone(), &fixture).await.unwrap(),
                PackageLoadOutcome::Updated(_)
            ));
            let before = store.snapshot().await.unwrap();
            candidate(&fixture, 10, next_snapshot, expires, 18);
            fs::remove_file(fixture.join(format!("{next_snapshot}.snapshot.json"))).unwrap();
            assert!(matches!(
                update(store.clone(), &fixture).await.unwrap(),
                PackageLoadOutcome::NoUpdate
            ));
            let after = store.snapshot().await.unwrap();
            assert_eq!(after.revision, before.revision);
            assert_eq!(after.accepted_time, before.accepted_time);
            assert_eq!(after.current_root, before.current_root);
            for (role, metadata) in before.metadata {
                assert_eq!(after.metadata[&role].bytes, metadata.bytes);
                assert_eq!(
                    after.metadata[&role].acceptance_root,
                    metadata.acceptance_root
                );
            }
        }
    }
}

#[tokio::test]
async fn signatures_rollback_and_newer_expiry_still_fail() {
    for (version, expires, seed) in [(10, FRESH, 19), (9, FRESH, 18), (11, EXPIRED, 18)] {
        let (_directory, store, fixture) = initial_view();
        update(store.clone(), &fixture).await.unwrap();
        let before = store.snapshot().await.unwrap();
        candidate(&fixture, version, 11, expires, seed);
        let error = *update(store.clone(), &fixture).await.unwrap_err();
        match version {
            10 => assert!(matches!(error, tough::error::Error::VerifyMetadata { .. })),
            9 => assert!(matches!(error, tough::error::Error::OlderMetadata { .. })),
            _ => assert!(matches!(error, tough::error::Error::ExpiredMetadata { .. })),
        }
        assert_eq!(store.snapshot().await.unwrap().revision, before.revision);
    }
}

#[tokio::test]
async fn newer_timestamp_still_loads_and_default_loader_still_accepts_changed_equal_version() {
    for use_package in [true, false] {
        let (_directory, store, fixture) = initial_view();
        update(store.clone(), &fixture).await.unwrap();
        candidate(&fixture, if use_package { 11 } else { 10 }, 11, FRESH, 18);
        let repo = if use_package {
            match update(store, &fixture).await.unwrap() {
                PackageLoadOutcome::Updated(repo) => repo,
                PackageLoadOutcome::NoUpdate => panic!("newer timestamp must update"),
            }
        } else {
            Box::new(
                load(
                    store,
                    &fixture,
                    ProbeAdmission::permit_for_storage_probe_only(),
                    "2026-01-01T00:00:00Z",
                )
                .await
                .unwrap(),
            )
        };
        assert_eq!(repo.targets().signed.version.to_string(), "11");
    }
}

#[tokio::test]
async fn no_update_does_not_require_or_return_a_complete_retained_view() {
    let (_directory, store, fixture) = initial_view();
    update(store.clone(), &fixture).await.unwrap();
    store
        .connection()
        .unwrap()
        .execute(
            "DELETE FROM metadata WHERE role!=?1",
            [serde_json::to_string(&MetadataRole::Timestamp).unwrap()],
        )
        .unwrap();
    candidate(&fixture, 10, 11, FRESH, 18);
    fs::remove_file(fixture.join("11.snapshot.json")).unwrap();
    assert!(matches!(
        update(store.clone(), &fixture).await.unwrap(),
        PackageLoadOutcome::NoUpdate
    ));
    assert_eq!(store.snapshot().await.unwrap().metadata.len(), 1);
}

#[tokio::test]
async fn root_advancement_survives_no_update_and_fresh_process_restart() {
    let (_directory, store, fixture) = initial_view();
    update(store.clone(), &fixture).await.unwrap();
    let before = store.snapshot().await.unwrap();
    let root2 = fixtures::root(2, 18);
    fs::write(fixture.join("2.root.json"), &root2).unwrap();
    candidate(&fixture, 10, 11, EXPIRED, 18);
    fs::remove_file(fixture.join("11.snapshot.json")).unwrap();
    assert!(matches!(
        update(store.clone(), &fixture).await.unwrap(),
        PackageLoadOutcome::NoUpdate
    ));
    let after = store.snapshot().await.unwrap();
    assert_eq!(after.current_root, root2);
    assert_eq!(after.revision, Revision(before.revision.0 + 2));
    assert!(after.reset_baseline.is_none());
    assert_eq!(
        after.metadata[&MetadataRole::Timestamp].bytes,
        before.metadata[&MetadataRole::Timestamp].bytes
    );
    fs::remove_file(fixture.join("2.root.json")).unwrap();
    assert!(Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "no_update::no_update_child"])
        .env("MORPHIR_TUF_NO_UPDATE_DB", &store.path)
        .env("MORPHIR_TUF_NO_UPDATE_FIXTURE", &fixture)
        .status()
        .unwrap()
        .success());
    assert_eq!(store.snapshot().await.unwrap().revision, after.revision);
}
#[tokio::test]
async fn no_update_child() {
    let Some(path) = std::env::var_os("MORPHIR_TUF_NO_UPDATE_DB") else {
        return;
    };
    let fixture = std::env::var_os("MORPHIR_TUF_NO_UPDATE_FIXTURE").unwrap();
    assert!(matches!(
        update(Arc::new(Sqlite { path: path.into() }), Path::new(&fixture))
            .await
            .unwrap(),
        PackageLoadOutcome::NoUpdate
    ));
}

#[tokio::test]
async fn failed_required_no_update_admission_cannot_emit_no_update() {
    let (_directory, store, fixture) = initial_view();
    update(store.clone(), &fixture).await.unwrap();
    let before = store.snapshot().await.unwrap();
    candidate(&fixture, 10, 11, FRESH, 18);
    let error = RepositoryLoader::new(
        &fs::read(fixture.join("1.root.json")).unwrap(),
        test_utils::dir_url(&fixture),
        test_utils::dir_url(&fixture),
    )
    .fixed_time("2026-01-01T00:00:00Z".parse().unwrap())
    .experimental_storage(
        store.clone(),
        Arc::new(ProbeAdmission {
            reject_begin: false,
            reject_transition: true,
        }),
    )
    .load_package()
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        tough::error::Error::ExperimentalStorage {
            source: StoreError::Admission
        }
    ));
    assert_eq!(store.snapshot().await.unwrap().revision, before.revision);
}

#[tokio::test]
async fn package_update_requires_explicit_transactional_context_and_safe_fixed_time() {
    let (_directory, store, fixture) = initial_view();
    let root = fs::read(fixture.join("1.root.json")).unwrap();
    let base = RepositoryLoader::new(
        &root,
        test_utils::dir_url(&fixture),
        test_utils::dir_url(&fixture),
    );
    let missing_storage = base.clone().load_package().await.unwrap_err();
    assert!(matches!(
        missing_storage,
        tough::error::Error::ExperimentalStorageConfiguration
    ));
    let configured = base.experimental_storage(
        store.clone(),
        Arc::new(ProbeAdmission::permit_for_storage_probe_only()),
    );
    let missing_time = configured.clone().load_package().await.unwrap_err();
    assert!(matches!(
        missing_time,
        tough::error::Error::ExperimentalStorageConfiguration
    ));
    let unsafe_time = configured
        .fixed_time("2026-01-01T00:00:00Z".parse().unwrap())
        .expiration_enforcement(tough::ExpirationEnforcement::Unsafe)
        .load_package()
        .await
        .unwrap_err();
    assert!(matches!(
        unsafe_time,
        tough::error::Error::FixedTimeRequiresExpirationEnforcement
    ));
    assert_eq!(store.snapshot().await.unwrap().revision, Revision(0));
}

#[tokio::test]
async fn retained_timestamp_must_still_authenticate_under_the_current_root_to_stop_update() {
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
    update(store.clone(), &fixture).await.unwrap();
    fs::write(fixture.join("2.root.json"), fixtures::threshold_root(2, 2)).unwrap();
    fixtures::write_threshold_view(&fixture, 10, &[18, 19]);
    assert!(matches!(
        update(store, &fixture).await.unwrap(),
        PackageLoadOutcome::Updated(_)
    ));
}

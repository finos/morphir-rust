//! Development-only storage-port conformance; no qualified package authorization.
use std::{fs, sync::Arc};
use tempfile::TempDir;
use tough::{
    experimental_storage::{Snapshot, Storage},
    RepositoryLoader,
};
mod storage_support;
mod test_utils;
use storage_support::Sqlite;

#[tokio::test]
async fn retained_root_is_authoritative_and_exact() {
    let directory = TempDir::new().unwrap();
    let fixture = test_utils::test_data().join("rotated-root");
    let root = fs::read(fixture.join("1.root.json")).unwrap();
    let store = Arc::new(Sqlite::initialize(
        &directory.path().join("state.db"),
        &root,
    ));
    let repo = RepositoryLoader::new(
        &root,
        test_utils::dir_url(&fixture),
        test_utils::dir_url(fixture.join("targets")),
    )
    .fixed_time("2026-01-01T00:00:00Z".parse().unwrap())
    .experimental_storage(
        store.clone(),
        Arc::new(storage_support::ProbeAdmission::permit_for_storage_probe_only()),
    )
    .load()
    .await
    .unwrap();
    assert_eq!(repo.root().signed.version.to_string(), "2");
    let persisted = store.snapshot().await.unwrap();
    assert_eq!(
        persisted.current_root,
        fs::read(fixture.join("2.root.json")).unwrap()
    );
    assert!(persisted.reset_baseline.is_none());
    assert_eq!(persisted.provisioned_root, root);
    assert_eq!(persisted.accepted_time, None);
    let _snapshot: Snapshot = persisted;
}

use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use storage_support::{fixtures, ProbeAdmission};
use tough::experimental_storage::{Error as StoreError, MetadataRole, Revision, Transition};

async fn load(
    store: Arc<Sqlite>,
    fixture: &Path,
    guard: ProbeAdmission,
    time: &str,
) -> Result<tough::Repository, Box<tough::error::Error>> {
    RepositoryLoader::new(
        &fs::read(fixture.join("1.root.json")).unwrap(),
        test_utils::dir_url(fixture),
        test_utils::dir_url(fixture),
    )
    .fixed_time(time.parse().unwrap())
    .experimental_storage(store, Arc::new(guard))
    .load()
    .await
    .map_err(Box::new)
}
fn initialized() -> (TempDir, Arc<Sqlite>, std::path::PathBuf) {
    let directory = TempDir::new().unwrap();
    let fixture = directory.path().join("fixture");
    fixtures::create(&fixture);
    let store = Arc::new(Sqlite::initialize(
        &directory.path().join("state.db"),
        &fixtures::root(1, 17),
    ));
    (directory, store, fixture)
}
fn seed_floor(store: &Sqlite, role: MetadataRole, bytes: &[u8]) {
    store
        .connection()
        .unwrap()
        .execute(
            "INSERT OR REPLACE INTO metadata VALUES(?1,?2)",
            rusqlite::params![serde_json::to_string(&role).unwrap(), bytes],
        )
        .unwrap();
}

#[tokio::test]
async fn required_admission_rejection_prevents_any_transition() {
    for begin in [true, false] {
        let (_directory, store, fixture) = initialized();
        assert!(load(
            store.clone(),
            &fixture,
            ProbeAdmission {
                reject_begin: begin,
                reject_transition: !begin
            },
            "2026-01-01T00:00:00Z"
        )
        .await
        .is_err());
        let snapshot = store.snapshot().await.unwrap();
        assert_eq!(snapshot.revision, Revision(0));
        assert_eq!(snapshot.current_root, fixtures::root(1, 17));
    }
}

#[tokio::test]
async fn later_failure_retains_exact_root_and_role_floors_without_advancing_time() {
    let (_directory, store, fixture) = initialized();
    store
        .connection()
        .unwrap()
        .execute("UPDATE state SET accepted='2020-01-01T00:00:00Z'", [])
        .unwrap();
    fixtures::write_view(&fixture, 50);
    fs::remove_file(fixture.join("50.targets.json")).unwrap();
    assert!(load(
        store.clone(),
        &fixture,
        ProbeAdmission::permit_for_storage_probe_only(),
        "2026-01-01T00:00:00Z"
    )
    .await
    .is_err());
    let state = store.snapshot().await.unwrap();
    let (timestamp, snapshot, _) = fixtures::view(50, 18);
    assert_eq!(state.current_root, fixtures::root(2, 18));
    assert_eq!(state.metadata[&MetadataRole::Timestamp], timestamp);
    assert_eq!(state.metadata[&MetadataRole::Snapshot], snapshot);
    assert_eq!(
        state.accepted_time.unwrap().to_string(),
        "2020-01-01T00:00:00Z"
    );
    fixtures::write_view(&fixture, 1);
    let result = load(
        store.clone(),
        &fixture,
        ProbeAdmission::permit_for_storage_probe_only(),
        "2026-01-01T00:00:00Z",
    )
    .await;
    assert!(matches!(
        *result.unwrap_err(),
        tough::error::Error::OlderMetadata { .. }
    ));
}

#[tokio::test]
async fn corrupt_protected_metadata_or_time_fails_closed() {
    for column in ["root", "baseline", "accepted", "timestamp", "snapshot"] {
        let (_directory, store, fixture) = initialized();
        match column {
            "timestamp" => seed_floor(&store, MetadataRole::Timestamp, b"malformed"),
            "snapshot" => seed_floor(&store, MetadataRole::Snapshot, b"malformed"),
            "accepted" => {
                store
                    .connection()
                    .unwrap()
                    .execute("UPDATE state SET accepted='malformed'", [])
                    .unwrap();
            }
            "baseline" => {
                store
                    .connection()
                    .unwrap()
                    .execute("UPDATE state SET baseline=X'00'", [])
                    .unwrap();
            }
            _ => {
                store
                    .connection()
                    .unwrap()
                    .execute("UPDATE state SET root=X'00'", [])
                    .unwrap();
            }
        }
        assert!(load(
            store.clone(),
            &fixture,
            ProbeAdmission::permit_for_storage_probe_only(),
            "2026-01-01T00:00:00Z"
        )
        .await
        .is_err());
        let revision: i64 = store
            .connection()
            .unwrap()
            .query_row("SELECT revision FROM state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(revision, 0);
    }
}

#[tokio::test]
async fn accepted_time_is_read_only_and_earlier_operations_are_rejected() {
    let (_directory, store, fixture) = initialized();
    store
        .connection()
        .unwrap()
        .execute("UPDATE state SET accepted='2026-01-01T00:00:00Z'", [])
        .unwrap();
    assert!(load(
        store.clone(),
        &fixture,
        ProbeAdmission::permit_for_storage_probe_only(),
        "2025-12-31T23:59:59Z"
    )
    .await
    .is_err());
    assert_eq!(store.snapshot().await.unwrap().revision, Revision(0));
    load(
        store.clone(),
        &fixture,
        ProbeAdmission::permit_for_storage_probe_only(),
        "2026-01-01T00:00:00Z",
    )
    .await
    .unwrap();
    assert_eq!(
        store
            .snapshot()
            .await
            .unwrap()
            .accepted_time
            .unwrap()
            .to_string(),
        "2026-01-01T00:00:00Z"
    );
}

#[tokio::test]
async fn stale_predecessor_cannot_commit() {
    let (_directory, store, _fixture) = initialized();
    let transition = Transition::AdvanceRoot {
        root: fixtures::root(2, 18),
        baseline: fixtures::root(1, 17),
    };
    store.commit(Revision(0), &transition).await.unwrap();
    assert!(matches!(
        store.commit(Revision(0), &transition).await,
        Err(StoreError::Conflict)
    ));
    let count: i64 = store
        .connection()
        .unwrap()
        .query_row("SELECT count(*) FROM roots", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[derive(Debug, Clone)]
struct ForbiddenTransport;
#[async_trait::async_trait]
impl tough::Transport for ForbiddenTransport {
    async fn fetch(&self, _: url::Url) -> Result<tough::TransportStream, tough::TransportError> {
        panic!("metadata transport must not run before admission");
    }
}
#[tokio::test]
async fn begin_rejection_prevents_metadata_transport() {
    let (_directory, store, fixture) = initialized();
    let root = fixtures::root(1, 17);
    let result = RepositoryLoader::new(
        &root,
        test_utils::dir_url(&fixture),
        test_utils::dir_url(&fixture),
    )
    .fixed_time("2026-01-01T00:00:00Z".parse().unwrap())
    .transport(ForbiddenTransport)
    .experimental_storage(
        store.clone(),
        Arc::new(ProbeAdmission {
            reject_begin: true,
            reject_transition: false,
        }),
    )
    .load()
    .await;
    assert!(matches!(
        result,
        Err(tough::error::Error::ExperimentalStorage {
            source: StoreError::Admission
        })
    ));
    let state = store.snapshot().await.unwrap();
    assert_eq!(state.revision, Revision(0));
    assert!(state.reset_baseline.is_none());
}

#[tokio::test]
async fn experimental_storage_rejects_ambiguous_or_unfixed_configuration() {
    let (directory, store, fixture) = initialized();
    let root = fixtures::root(1, 17);
    for mode in ["no-clock", "directory", "unsafe"] {
        let loader = RepositoryLoader::new(
            &root,
            test_utils::dir_url(&fixture),
            test_utils::dir_url(&fixture),
        )
        .transport(ForbiddenTransport)
        .experimental_storage(
            store.clone(),
            Arc::new(ProbeAdmission::permit_for_storage_probe_only()),
        );
        let loader = match mode {
            "no-clock" => loader,
            "directory" => loader
                .fixed_time("2026-01-01T00:00:00Z".parse().unwrap())
                .datastore(directory.path()),
            _ => loader
                .fixed_time("2026-01-01T00:00:00Z".parse().unwrap())
                .expiration_enforcement(tough::ExpirationEnforcement::Unsafe),
        };
        assert!(loader.load().await.is_err());
        assert_eq!(store.snapshot().await.unwrap().revision, Revision(0));
    }
}

#[path = "storage_support/restarts.rs"]
mod restarts;

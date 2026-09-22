use jiff::Timestamp;
use tempfile::TempDir;
use test_utils::{dir_url, test_data};
use tough::{
    error::Error, schema::RoleType, ExpirationEnforcement, IntoVec, Repository, RepositoryLoader,
    TargetName,
};
mod test_utils;

async fn load_at(time: &str, store: &TempDir) -> Result<Repository, Box<Error>> {
    load_fixture_at("expired-repository", time, store).await
}

async fn load_fixture_at(
    fixture: &str,
    time: &str,
    store: &TempDir,
) -> Result<Repository, Box<Error>> {
    let base = test_data().join(fixture);
    RepositoryLoader::new(
        &std::fs::read(base.join("metadata/1.root.json")).unwrap(),
        dir_url(base.join("metadata")),
        dir_url(base.join("targets")),
    )
    .datastore(store.path())
    .fixed_time(time.parse().unwrap())
    .load()
    .await
    .map_err(Box::new)
}

#[tokio::test]
async fn fixed_time_enforces_before_equal_after_expiration() {
    for (time, expired) in [
        ("1998-12-31T23:59:59Z", false),
        ("1999-01-01T00:00:00Z", true),
        ("1999-01-01T00:00:01Z", true),
    ] {
        let result = load_at(time, &TempDir::new().unwrap()).await;
        if expired {
            assert!(
                matches!(
                    result.as_ref().map_err(|error| error.as_ref()),
                    Err(Error::ExpiredMetadata {
                        role: RoleType::Timestamp,
                        ..
                    })
                ),
                "unexpected result: {:?}",
                result
            );
        } else {
            assert!(result.is_ok(), "unexpected result: {:?}", result);
        }
    }
}

#[tokio::test]
async fn fixed_time_is_retained_for_target_reads_and_known_time() {
    let store = TempDir::new().unwrap();
    let time = "1998-12-31T23:59:59Z";
    let repo = load_fixture_at("tuf-reference-impl", time, &store)
        .await
        .unwrap();
    for name in ["file1.txt", "file2.txt", "file3.txt", "file1.txt"] {
        let target = repo
            .read_target(&TargetName::new(name).unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(!target.into_vec().await.unwrap().is_empty());
        let recorded: Timestamp = serde_json::from_slice(
            &std::fs::read(store.path().join("latest_known_time.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(recorded, time.parse::<Timestamp>().unwrap());
    }
}

#[tokio::test]
async fn fixed_time_uses_existing_rollback_check_on_load_and_read() {
    let store = TempDir::new().unwrap();
    let repo = load_at("1998-12-31T23:59:59Z", &store).await.unwrap();
    assert!(matches!(
        *load_at("1998-12-31T23:59:58Z", &store).await.unwrap_err(),
        Error::SystemTimeSteppedBackward { .. }
    ));
    std::fs::write(
        store.path().join("latest_known_time.json"),
        br#""1999-01-01T00:00:00Z""#,
    )
    .unwrap();
    assert!(matches!(
        repo.read_target(&TargetName::new("file1.txt").unwrap())
            .await,
        Err(Error::SystemTimeSteppedBackward { .. })
    ));
}

#[tokio::test]
async fn fixed_time_cannot_disable_expiration_in_either_builder_order() {
    let base = test_data().join("expired-repository");
    let root = std::fs::read(base.join("metadata/1.root.json")).unwrap();
    for before in [true, false] {
        let loader = RepositoryLoader::new(
            &root,
            dir_url(base.join("metadata")),
            dir_url(base.join("targets")),
        );
        let time: Timestamp = "1998-12-31T23:59:59Z".parse().unwrap();
        let loader = if before {
            loader
                .expiration_enforcement(ExpirationEnforcement::Unsafe)
                .fixed_time(time)
        } else {
            loader
                .fixed_time(time)
                .expiration_enforcement(ExpirationEnforcement::Unsafe)
        };
        assert!(matches!(
            loader.load().await,
            Err(Error::FixedTimeRequiresExpirationEnforcement)
        ));
    }
}

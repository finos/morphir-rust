//! Independently signed version-boundary vectors for the package-local TUF candidate.
//! The candidate datastore is tested for exact numeric replay, not production durability.
use ed25519_zebra::{SigningKey, VerificationKeyBytes};
use package_tough::{
    RepositoryLoader,
    schema::{Root, Signed, Targets},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

const MAX: &str = "18446744073709551615";
const NEXT: &str = "18446744073709551616";
const LATER: &str = "18446744073709551617";
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn number(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}
fn canonical(value: &Value) -> Vec<u8> {
    let mut ordered = value.clone();
    ordered.sort_all_objects();
    serde_json::to_vec(&ordered).unwrap()
}
fn signer() -> SigningKey {
    SigningKey::from([17; 32])
}
fn key() -> Value {
    json!({"keytype":"ed25519","scheme":"ed25519","keyval":{"public":hex(VerificationKeyBytes::from(&signer()).as_ref())}})
}
fn key_id() -> String {
    hex(&Sha256::digest(canonical(&key())))
}
fn signed(body: Value) -> Value {
    // ASCII strings and exact integers only: the ordered serde_json map emits the
    // canonical signed bytes independently of the candidate's serializer/verifier.
    let signature = signer().sign(&canonical(&body));
    json!({"signed":body,"signatures":[{"keyid":key_id(),"sig":hex(&signature.to_bytes())}]})
}
fn root(version: &str) -> Value {
    let role = json!({"keyids":[key_id()],"threshold":1});
    signed(
        json!({"_type":"root","spec_version":"1.0.36","version":number(version),"expires":"2100-01-01T00:00:00Z","consistent_snapshot":true,"keys":{key_id():key()},"roles":{"root":role,"timestamp":role,"snapshot":role,"targets":role}}),
    )
}
fn targets(version: &str) -> Value {
    signed(
        json!({"_type":"targets","spec_version":"1.0.36","version":number(version),"expires":"2100-01-01T00:00:00Z","targets":{}}),
    )
}
fn description(document: &Value, version: &str) -> Value {
    let bytes = canonical(document);
    json!({"version":number(version),"length":bytes.len(),"hashes":{"sha256":hex(&Sha256::digest(bytes))}})
}
fn write_view(directory: &Path, version: &str) {
    fs::create_dir_all(directory.join("metadata")).unwrap();
    fs::create_dir_all(directory.join("targets")).unwrap();
    let targets = targets(version);
    let snapshot = signed(
        json!({"_type":"snapshot","spec_version":"1.0.36","version":number(version),"expires":"2100-01-01T00:00:00Z","meta":{"targets.json":description(&targets,version)}}),
    );
    let timestamp = signed(
        json!({"_type":"timestamp","spec_version":"1.0.36","version":number(version),"expires":"2100-01-01T00:00:00Z","meta":{"snapshot.json":description(&snapshot,version)}}),
    );
    for (name, document) in [
        (format!("{version}.targets.json"), targets),
        (format!("{version}.snapshot.json"), snapshot),
        ("timestamp.json".into(), timestamp),
    ] {
        fs::write(directory.join("metadata").join(name), canonical(&document)).unwrap();
    }
}
async fn load(
    directory: &Path,
    bootstrap: &[u8],
    store: &Path,
) -> Result<package_tough::Repository, Box<package_tough::error::Error>> {
    RepositoryLoader::new(
        &bootstrap,
        url::Url::from_directory_path(directory.join("metadata")).unwrap(),
        url::Url::from_directory_path(directory.join("targets")).unwrap(),
    )
    .datastore(store)
    .load()
    .await
    .map_err(Box::new)
}

#[test]
fn signed_versions_across_u64_boundary_preserve_authentication() {
    for version in [
        "1",
        "9007199254740993",
        MAX,
        NEXT,
        LATER,
        "99999999999999999999999999999999999999",
    ] {
        let trust: Signed<Root> = serde_json::from_slice(&canonical(&root("1"))).unwrap();
        let original = targets(version);
        let parsed = serde_json::from_slice::<Signed<Targets>>(&canonical(&original));
        assert!(
            parsed.is_ok(),
            "valid signed version {version} rejected: {parsed:?}"
        );
        let parsed = parsed.unwrap();
        trust.signed.verify_role(&parsed).unwrap();
        assert_eq!(
            serde_json::to_value(&parsed).unwrap()["signed"],
            original["signed"]
        );
        let mut tampered = original;
        tampered["signed"]["version"] = number("2");
        let parsed: Signed<Targets> = serde_json::from_slice(&canonical(&tampered)).unwrap();
        assert!(trust.signed.verify_role(&parsed).is_err());
    }
}

#[tokio::test]
async fn exact_successor_crosses_u64_and_uses_full_decimal_filenames() {
    let directory = TempDir::new().unwrap();
    let store = TempDir::new().unwrap();
    write_view(directory.path(), NEXT);
    let bootstrap = canonical(&root(MAX));
    fs::write(
        directory.path().join(format!("metadata/{NEXT}.root.json")),
        canonical(&root(NEXT)),
    )
    .unwrap();
    let repository = load(directory.path(), &bootstrap, store.path())
        .await
        .unwrap();
    assert_eq!(repository.root().signed.version.to_string(), NEXT);
    assert_eq!(repository.targets().signed.version.to_string(), NEXT);
    drop(repository);
    let retained_root = fs::read(store.path().join("root.json")).unwrap();
    let retained: Signed<Root> = serde_json::from_slice(&retained_root).unwrap();
    retained.signed.verify_role(&retained).unwrap();
    assert_eq!(retained.signed.version.to_string(), NEXT);
    fs::write(directory.path().join("bootstrap.json"), retained_root).unwrap();
    child(directory.path(), store.path(), "accept");
}

#[tokio::test]
async fn signed_root_with_skipped_successor_is_rejected() {
    let directory = TempDir::new().unwrap();
    let store = TempDir::new().unwrap();
    write_view(directory.path(), "1");
    fs::write(
        directory.path().join(format!("metadata/{NEXT}.root.json")),
        canonical(&root(LATER)),
    )
    .unwrap();
    let error = load(directory.path(), &canonical(&root(MAX)), store.path())
        .await
        .unwrap_err();
    assert!(
        matches!(
            *error,
            package_tough::error::Error::OlderMetadata {
                role: package_tough::schema::RoleType::Root,
                ..
            }
        ),
        "{error}"
    );
}

#[tokio::test]
async fn committed_large_versions_survive_a_fresh_process_and_reject_rollback() {
    let directory = TempDir::new().unwrap();
    let store = TempDir::new().unwrap();
    let bootstrap = canonical(&root("1"));
    fs::write(directory.path().join("bootstrap.json"), &bootstrap).unwrap();
    write_view(directory.path(), NEXT);
    drop(
        load(directory.path(), &bootstrap, store.path())
            .await
            .unwrap(),
    );
    child(directory.path(), store.path(), "accept");
    write_view(directory.path(), MAX);
    child(directory.path(), store.path(), "reject");
    let timestamp: Value =
        serde_json::from_slice(&fs::read(store.path().join("timestamp.json")).unwrap()).unwrap();
    assert_eq!(timestamp["signed"]["version"], number(NEXT));
}
fn child(directory: &Path, store: &Path, outcome: &str) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "version_restart_child", "--nocapture"])
        .env("MORPHIR_TUF_VERSION_TEST_DIRECTORY", directory)
        .env("MORPHIR_TUF_VERSION_TEST_STORE", store)
        .env("MORPHIR_TUF_VERSION_TEST_OUTCOME", outcome)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[tokio::test]
async fn version_restart_child() {
    let Some(directory) = std::env::var_os("MORPHIR_TUF_VERSION_TEST_DIRECTORY") else {
        return;
    };
    let directory = Path::new(&directory);
    let store = std::env::var_os("MORPHIR_TUF_VERSION_TEST_STORE").unwrap();
    let bootstrap = fs::read(directory.join("bootstrap.json")).unwrap();
    let result = load(directory, &bootstrap, Path::new(&store)).await;
    match std::env::var("MORPHIR_TUF_VERSION_TEST_OUTCOME")
        .unwrap()
        .as_str()
    {
        "accept" => {
            let repository = result.unwrap();
            assert_eq!(repository.targets().signed.version.to_string(), NEXT);
            let expected: Value = serde_json::from_slice(&bootstrap).unwrap();
            assert_eq!(
                repository.root().signed.version.to_string(),
                expected["signed"]["version"].to_string()
            );
        }
        "reject" => {
            let error = result.unwrap_err();
            assert!(
                matches!(
                    *error,
                    package_tough::error::Error::OlderMetadata {
                        role: package_tough::schema::RoleType::Timestamp,
                        ..
                    }
                ),
                "{error}"
            );
        }
        other => panic!("unexpected child outcome {other}"),
    }
}

#[tokio::test]
async fn higher_timestamp_cannot_roll_back_a_large_snapshot_version() {
    let directory = TempDir::new().unwrap();
    let store = TempDir::new().unwrap();
    let bootstrap = canonical(&root("1"));
    write_view(directory.path(), NEXT);
    drop(
        load(directory.path(), &bootstrap, store.path())
            .await
            .unwrap(),
    );
    write_view(directory.path(), MAX);
    let path = directory.path().join("metadata/timestamp.json");
    let mut timestamp: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    timestamp["signed"]["version"] = number(LATER);
    fs::write(path, canonical(&signed(timestamp["signed"].clone()))).unwrap();
    let error = load(directory.path(), &bootstrap, store.path())
        .await
        .unwrap_err();
    assert!(
        matches!(
            *error,
            package_tough::error::Error::OlderSnapshotInTimestamp { .. }
        ),
        "{error}"
    );
}

#[tokio::test]
async fn root_update_budget_counts_transitions_independently_of_version_magnitude() {
    let directory = TempDir::new().unwrap();
    let store = TempDir::new().unwrap();
    write_view(directory.path(), "1");
    for version in [NEXT, LATER] {
        fs::write(
            directory
                .path()
                .join(format!("metadata/{version}.root.json")),
            canonical(&root(version)),
        )
        .unwrap();
    }
    let bootstrap = canonical(&root(MAX));
    let result = RepositoryLoader::new(
        &bootstrap,
        url::Url::from_directory_path(directory.path().join("metadata")).unwrap(),
        url::Url::from_directory_path(directory.path().join("targets")).unwrap(),
    )
    .datastore(store.path())
    .limits(package_tough::Limits {
        max_root_updates: 1,
        ..Default::default()
    })
    .load()
    .await;
    let error = result.unwrap_err();
    assert!(
        matches!(
            error,
            package_tough::error::Error::MaxUpdatesExceeded { .. }
        ),
        "{error}"
    );
}

#[tokio::test]
async fn higher_timestamp_and_snapshot_cannot_roll_back_large_targets_version() {
    let directory = TempDir::new().unwrap();
    let store = TempDir::new().unwrap();
    let bootstrap = canonical(&root("1"));
    write_view(directory.path(), LATER);
    drop(
        load(directory.path(), &bootstrap, store.path())
            .await
            .unwrap(),
    );
    write_view(directory.path(), NEXT);
    let newer = "18446744073709551618";
    let snapshot = signed(
        json!({"_type":"snapshot","spec_version":"1.0.36","version":number(newer),"expires":"2100-01-01T00:00:00Z","meta":{"targets.json":description(&targets(NEXT),NEXT)}}),
    );
    let timestamp = signed(
        json!({"_type":"timestamp","spec_version":"1.0.36","version":number(newer),"expires":"2100-01-01T00:00:00Z","meta":{"snapshot.json":description(&snapshot,newer)}}),
    );
    fs::write(
        directory
            .path()
            .join(format!("metadata/{newer}.snapshot.json")),
        canonical(&snapshot),
    )
    .unwrap();
    fs::write(
        directory.path().join("metadata/timestamp.json"),
        canonical(&timestamp),
    )
    .unwrap();
    let error = load(directory.path(), &bootstrap, store.path())
        .await
        .unwrap_err();
    assert!(
        matches!(
            *error,
            package_tough::error::Error::SnapshotRoleRollback { .. }
        ),
        "{error}"
    );
}

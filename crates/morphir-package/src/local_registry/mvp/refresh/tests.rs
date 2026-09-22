use super::*;
use std::fs;
#[path = "../../../../tests/local_registry/tuf_mothers.rs"]
#[allow(dead_code)]
mod tuf_mothers;

#[tokio::test]
async fn repeated_timestamp_rechecks_retained_root_and_child_expiry_at_the_operation_time() {
    for expired in ["root", "snapshot", "targets"] {
        let dir = tempfile::tempdir().unwrap();
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/local_registry/mvp-fixture");
        let policy = fs::read_to_string(fixture.join("trust-policy.json"))
            .unwrap()
            .replace("previous-authorization", "fresh-metadata")
            .into_bytes();
        fs::create_dir_all(dir.path().join("registry/metadata")).unwrap();
        for entry in fs::read_dir(fixture.join("registry/metadata")).unwrap() {
            let entry = entry.unwrap();
            fs::copy(
                entry.path(),
                dir.path().join("registry/metadata").join(entry.file_name()),
            )
            .unwrap();
        }
        fs::write(
            dir.path().join("registry/metadata/1.root.json"),
            tuf_mothers::root(1, 42),
        )
        .unwrap();

        let registry = dir.path().join("registry");
        let mut prior: Option<Vec<u8>> = None;
        for (role, child) in [
            ("root", None),
            ("targets", None),
            ("snapshot", Some("targets.json")),
            ("timestamp", Some("snapshot.json")),
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(
                &fs::read(registry.join(format!("metadata/1.{role}.json"))).unwrap(),
            )
            .unwrap();
            value["signed"]["expires"] = serde_json::json!(if role == expired {
                "2028-01-01T00:00:00Z"
            } else {
                "2100-01-01T00:00:00Z"
            });
            if let Some(child) = child {
                value["signed"]["meta"][child] = tuf_mothers::meta(prior.as_ref().unwrap(), 1);
            }
            let bytes = tuf_mothers::sign(value["signed"].clone(), &[(42, tuf_mothers::key(42))]);
            fs::write(registry.join(format!("metadata/1.{role}.json")), &bytes).unwrap();
            if role == "timestamp" {
                fs::write(registry.join("metadata/timestamp.json"), &bytes).unwrap();
            }
            prior = Some(bytes);
        }
        let root = fs::read(registry.join("metadata/1.root.json")).unwrap();
        let mut policy: serde_json::Value = serde_json::from_slice(&policy).unwrap();
        policy["repositories"][0]["bootstrapRoot"]["digest"] =
            serde_json::json!(crate::digest::Digest::of_bytes(&root).to_string());
        let policy = serde_json::to_vec(&policy).unwrap();
        let state = dir.path().join("expiry-trust");
        initialize(InitializeRequest {
            policy: &policy,
            root: &root,
            state: &state,
        })
        .unwrap();
        refresh_at(
            RefreshRequest {
                policy: &policy,
                registry: &registry,
                state: &state,
            },
            "2027-01-01T00:00:00Z".parse().unwrap(),
        )
        .await
        .unwrap();
        let error = refresh_at(
            RefreshRequest {
                policy: &policy,
                registry: &registry,
                state: &state,
            },
            "2029-01-01T00:00:00Z".parse().unwrap(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("expired"), "{expired}: {error}");
        assert!(state.join("operation").exists());
        let db = rusqlite::Connection::open(state.join("trust.sqlite")).unwrap();
        let accepted: String = db
            .query_row("SELECT accepted FROM state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(accepted, "2027-01-01T00:00:00Z");
    }
}

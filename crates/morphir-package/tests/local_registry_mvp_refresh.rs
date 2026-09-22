use morphir_package::local_registry::mvp::{self, InitializeRequest, RefreshRequest};
use std::fs;
#[path = "local_registry/resolve_mothers.rs"]
#[allow(dead_code)]
mod mothers;
#[path = "local_registry/tuf_mothers.rs"]
#[allow(dead_code)]
mod tuf_mothers;

#[tokio::test]
async fn refresh_reports_exact_fixed_envelope_digests_and_repeats_without_artifacts() {
    let directory = tempfile::tempdir().unwrap();
    let policy = fs::read_to_string(mothers::fixture().join("trust-policy.json"))
        .unwrap()
        .replace("previous-authorization", "fresh-metadata")
        .into_bytes();
    let root = fs::read(mothers::fixture().join("registry/metadata/1.root.json")).unwrap();
    let state = directory.path().join("trust");
    mvp::initialize(InitializeRequest {
        policy: &policy,
        root: &root,
        state: &state,
    })
    .unwrap();
    for _ in 0..2 {
        let report = mvp::refresh(RefreshRequest {
            policy: &policy,
            registry: &mothers::fixture().join("registry"),
            state: &state,
        })
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(report).unwrap(),
            serde_json::json!({
                "profile":"local-library-mvp", "profileVersion":"0.1.0-draft.1", "registry":"local",
                "timestampDigest":"sha256:86a1dc9216e9bf51c7f077b59af785d38796c1c21d55c3faa8b7311571003bc0",
                "snapshotDigest":"sha256:8e77cde272b228a6c84293566ceefa4d3fa7c32c0e3b66e9524eb28763388bdb"
            })
        );
        assert!(!state.join("operation").exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}

async fn refresh(dir: &tempfile::TempDir, policy: &[u8]) -> Result<mvp::RefreshReport, mvp::Error> {
    mvp::refresh(RefreshRequest {
        policy,
        registry: &dir.path().join("registry"),
        state: &dir.path().join("trust"),
    })
    .await
}

#[tokio::test]
async fn revoked_declarations_refuse_without_package_reads_and_repaired_input_cannot_clear_marker()
{
    let (dir, policy) = mothers::registry_with_targets(|targets, registry| {
        targets["records/eligibility-1.2.0.json"]["custom"]["morphir"]["status"] =
            serde_json::json!("revoked");
        fs::remove_dir_all(registry.join("targets")).unwrap();
        fs::remove_dir_all(registry.join("bundles")).unwrap();
    });
    let error = refresh(&dir, &policy).await.unwrap_err();
    assert!(error.to_string().contains("revocation"), "{error}");
    assert!(dir.path().join("trust/operation").exists());
    let (active, _) = mothers::registry_with_targets(|_, _| {});
    for entry in fs::read_dir(active.path().join("registry/metadata")).unwrap() {
        let entry = entry.unwrap();
        fs::copy(
            entry.path(),
            dir.path().join("registry/metadata").join(entry.file_name()),
        )
        .unwrap();
    }
    let error = refresh(&dir, &policy).await.unwrap_err();
    assert!(
        error.to_string().contains("unresolved prior operation"),
        "{error}"
    );
}

#[tokio::test]
async fn malformed_target_declarations_refuse_before_selection() {
    for fault in ["status", "release", "kind", "extra", "path", "limit"] {
        let (dir, policy) = mothers::registry_with_targets(|targets, _| {
            let custom = &mut targets["records/eligibility-1.2.0.json"]["custom"]["morphir"];
            match fault {
                "status" => custom["status"] = serde_json::json!("unknown"),
                "release" => custom["release"]["version"] = serde_json::json!("bad"),
                "kind" => custom["kind"] = serde_json::json!("Executable"),
                "extra" => custom["extra"] = serde_json::json!(true),
                "path" => {
                    let value = targets["records/eligibility-1.2.0.json"].clone();
                    targets["other/record.json"] = value;
                }
                "limit" => {
                    let value = targets["records/eligibility-1.2.0.json"].clone();
                    for i in 0..4096 {
                        targets[format!("records/extra-{i}.json")] = value.clone();
                    }
                }
                _ => unreachable!(),
            }
        });
        let error = refresh(&dir, &policy).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains(if fault == "limit" { "limit" } else { "target" }),
            "{fault}: {error}"
        );
        assert!(dir.path().join("trust/operation").exists());
    }
}

#[tokio::test]
async fn missing_or_corrupt_package_material_is_ignored_but_restore_still_refuses() {
    use morphir_package::local_registry::mvp::{ResolveRequest, RestoreRequest};
    for fault in [
        "missing-bundle",
        "corrupt-bundle",
        "missing-statement",
        "corrupt-statement",
        "missing-record",
    ] {
        let (dir, policy) = mothers::registry_with_targets(|_, _| {});
        let registry = dir.path().join("registry");
        let state = dir.path().join("trust");
        let output = dir.path().join("morphir.lock");
        mvp::resolve(ResolveRequest {
            policy: &policy,
            root: mothers::root(),
            registry: &registry,
            state: &state,
            output: &output,
        })
        .await
        .unwrap();
        let original_lock = fs::read(&output).unwrap();
        let bundle = registry
            .join("bundles/5922bc8860f6cd008b9cda341be7f3a776ea332e63261392e94c17e19a647886");
        match fault {
            "missing-bundle" => fs::remove_dir_all(bundle).unwrap(),
            "corrupt-bundle" => fs::write(bundle.join("ir.json"), b"corrupt").unwrap(),
            "missing-record" => fs::remove_dir_all(registry.join("targets/records")).unwrap(),
            "missing-statement" => fs::remove_dir_all(registry.join("targets/statements")).unwrap(),
            "corrupt-statement" => {
                for entry in fs::read_dir(registry.join("targets/statements")).unwrap() {
                    fs::write(entry.unwrap().path(), b"corrupt").unwrap();
                }
            }
            _ => unreachable!(),
        }
        refresh(&dir, &policy).await.unwrap();
        assert_eq!(fs::read(&output).unwrap(), original_lock);
        assert!(!state.join("operation").exists());
        let restored = dir.path().join("restored");
        assert!(
            mvp::restore(RestoreRequest {
                policy: &policy,
                lock: &original_lock,
                registry: &registry,
                state: &state,
                output: &restored
            })
            .await
            .is_err(),
            "{fault}"
        );
        assert!(!restored.exists());
    }
}

/// Independent signer advances all linked versions, preserving exact decimal tokens.
fn advance(registry: &std::path::Path, version: &str, fault: Option<&str>) -> (String, String) {
    let version: serde_json::Value = serde_json::from_str(version).unwrap();
    let mut prior: Option<Vec<u8>> = None;
    let mut digests = Vec::new();
    for (role, child) in [
        ("targets", None),
        ("snapshot", Some("targets.json")),
        ("timestamp", Some("snapshot.json")),
    ] {
        let mut value: serde_json::Value = serde_json::from_slice(
            &fs::read(registry.join(format!("metadata/1.{role}.json"))).unwrap(),
        )
        .unwrap();
        value["signed"]["version"] = version.clone();
        if fault == Some(role) {
            value["signed"]["expires"] = serde_json::json!("2020-01-01T00:00:00Z");
        }
        if let Some(child) = child {
            let mut link = tuf_mothers::meta(prior.as_ref().unwrap(), 1);
            link["version"] = version.clone();
            if fault == Some("link") && role == "timestamp" {
                link["hashes"]["sha256"] = serde_json::json!("00".repeat(32));
            }
            value["signed"]["meta"][child] = link;
        }
        let bytes = tuf_mothers::sign(value["signed"].clone(), &[(42, tuf_mothers::key(42))]);
        fs::write(
            registry.join(format!("metadata/{version}.{role}.json")),
            &bytes,
        )
        .unwrap();
        if role == "timestamp" {
            fs::write(registry.join("metadata/timestamp.json"), &bytes).unwrap();
        }
        digests.push(format!(
            "sha256:{}",
            tuf_mothers::hex(<sha2::Sha256 as sha2::Digest>::digest(&bytes))
        ));
        prior = Some(bytes);
    }
    (digests[2].clone(), digests[1].clone())
}

#[tokio::test]
async fn signed_advance_retains_rollback_floors_including_versions_above_u64() {
    for version in ["2", "18446744073709551616"] {
        let (dir, policy) = mothers::registry_with_targets(|_, _| {});
        let registry = dir.path().join("registry");
        refresh(&dir, &policy).await.unwrap();
        let old = fs::read(registry.join("metadata/timestamp.json")).unwrap();
        let (timestamp, snapshot) = advance(&registry, version, None);
        for _ in 0..2 {
            let report = refresh(&dir, &policy).await.unwrap();
            assert_eq!(report.timestamp_digest.as_str(), timestamp);
            assert_eq!(report.snapshot_digest.as_str(), snapshot);
        }
        fs::write(registry.join("metadata/timestamp.json"), old).unwrap();
        let error = refresh(&dir, &policy).await.unwrap_err();
        assert!(
            error.to_string().to_lowercase().contains("rollback")
                || error.to_string().contains("previously fetched"),
            "{error}"
        );
        assert!(dir.path().join("trust/operation").exists());
    }
}

#[tokio::test]
async fn bad_signatures_expiry_and_links_never_return_reports_and_restarts_refuse() {
    for fault in ["signature", "timestamp", "snapshot", "targets", "link"] {
        let (dir, policy) = mothers::registry_with_targets(|_, _| {});
        let registry = dir.path().join("registry");
        if fault == "signature" {
            let path = registry.join("metadata/timestamp.json");
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            value["signatures"][0]["sig"] = serde_json::json!("00".repeat(64));
            fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        } else {
            advance(&registry, "2", Some(fault));
        }
        let error = refresh(&dir, &policy).await.unwrap_err();
        let message = error.to_string().to_lowercase();
        assert!(
            message.contains(match fault {
                "signature" => "signature",
                "link" => "hash",
                _ => "expir",
            }),
            "{fault}: {error}"
        );
        assert!(dir.path().join("trust/operation").exists());
        let error = refresh(&dir, &policy).await.unwrap_err();
        assert!(
            error.to_string().contains("unresolved prior operation"),
            "{error}"
        );
    }
}

#[tokio::test]
async fn equal_timestamp_reauthenticates_current_snapshot_and_targets_bytes() {
    for role in ["snapshot", "targets"] {
        let (dir, policy) = mothers::registry_with_targets(|_, _| {});
        refresh(&dir, &policy).await.unwrap();
        fs::write(
            dir.path().join(format!("registry/metadata/1.{role}.json")),
            b"corrupt",
        )
        .unwrap();
        assert!(refresh(&dir, &policy).await.is_err(), "{role}");
        assert!(dir.path().join("trust/operation").exists());
    }
}

#[tokio::test]
async fn missing_corrupt_deleted_and_unresolved_state_never_reset_trust() {
    for fault in ["directory", "database", "corrupt", "rows", "marker"] {
        let (dir, policy) = mothers::registry_with_targets(|_, _| {});
        refresh(&dir, &policy).await.unwrap();
        let state = dir.path().join("trust");
        match fault {
            "directory" => fs::remove_dir_all(&state).unwrap(),
            "database" => fs::remove_file(state.join("trust.sqlite")).unwrap(),
            "corrupt" => fs::write(state.join("trust.sqlite"), b"corrupt").unwrap(),
            "rows" => {
                let db = rusqlite::Connection::open(state.join("trust.sqlite")).unwrap();
                db.execute("DELETE FROM metadata", []).unwrap();
            }
            "marker" => fs::write(state.join("operation"), b"interrupted").unwrap(),
            _ => unreachable!(),
        }
        assert!(refresh(&dir, &policy).await.is_err(), "{fault}");
    }
}

#[tokio::test]
async fn concurrent_lock_and_failed_sqlite_commits_refuse() {
    let (dir, policy) = mothers::registry_with_targets(|_, _| {});
    let state = dir.path().join("trust");
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(state.join("lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&held).unwrap();
    let error = refresh(&dir, &policy).await.unwrap_err();
    assert!(error.to_string().contains("locked by another"), "{error}");
    assert!(!state.join("operation").exists());
    drop(held);
    for trigger in ["UPDATE ON state", "UPDATE OF accepted ON state"] {
        let (dir, policy) = mothers::registry_with_targets(|_, _| {});
        let state = dir.path().join("trust");
        let db = rusqlite::Connection::open(state.join("trust.sqlite")).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER fail_write BEFORE {trigger} BEGIN SELECT RAISE(FAIL, 'injected write failure'); END;")).unwrap();
        drop(db);
        let error = refresh(&dir, &policy).await.unwrap_err();
        assert!(
            error.to_string().contains("injected write failure"),
            "{error}"
        );
        assert!(state.join("operation").exists());
        let db = rusqlite::Connection::open(state.join("trust.sqlite")).unwrap();
        db.execute_batch("DROP TRIGGER fail_write").unwrap();
        drop(db);
        assert!(
            refresh(&dir, &policy)
                .await
                .unwrap_err()
                .to_string()
                .contains("unresolved prior operation")
        );
    }
}

#[tokio::test]
async fn refresh_advances_metadata_without_rewriting_old_lock_evidence() {
    use morphir_package::local_registry::mvp::{ResolveRequest, RestoreRequest};
    let (dir, policy) = mothers::registry_with_targets(|_, _| {});
    let registry = dir.path().join("registry");
    let state = dir.path().join("trust");
    let output = dir.path().join("morphir.lock");
    mvp::resolve(ResolveRequest {
        policy: &policy,
        root: mothers::root(),
        registry: &registry,
        state: &state,
        output: &output,
    })
    .await
    .unwrap();
    let original = fs::read(&output).unwrap();
    advance(&registry, "2", None);
    refresh(&dir, &policy).await.unwrap();
    assert_eq!(fs::read(&output).unwrap(), original);
    let error = mvp::restore(RestoreRequest {
        policy: &policy,
        lock: &original,
        registry: &registry,
        state: &state,
        output: &dir.path().join("restored"),
    })
    .await
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("historical evidence unsupported"),
        "{error}"
    );
    assert!(!dir.path().join("restored").exists());
}

#[tokio::test]
async fn refresh_accepts_yanked_declarations_without_changing_registry_files() {
    use std::{
        collections::BTreeMap,
        path::{Path, PathBuf},
    };
    fn bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fs::read_dir(root)
            .unwrap()
            .flat_map(|entry| {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    bytes(&path)
                } else {
                    BTreeMap::from([(path.clone(), fs::read(&path).unwrap())])
                }
            })
            .collect()
    }
    let (dir, policy) = mothers::registry_with_targets(|targets, _| {
        targets["records/eligibility-1.2.0.json"]["custom"]["morphir"]["status"] =
            serde_json::json!("yanked");
    });
    let registry = dir.path().join("registry");
    let before = bytes(&registry);
    refresh(&dir, &policy).await.unwrap();
    assert_eq!(bytes(&registry), before);
}

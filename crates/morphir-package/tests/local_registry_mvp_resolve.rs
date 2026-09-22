use morphir_package::{
    local_registry::{
        decode_library_lock,
        mvp::{self, InitializeRequest, ResolveRequest, RestoreRequest},
    },
    resolution::{PackagePath, ReleaseId, StableVersion},
};
use std::fs;
#[path = "local_registry/resolve_mothers.rs"]
mod mothers;
use mothers::{fixture, registry_with_targets, root};
#[path = "local_registry/tuf_mothers.rs"]
#[allow(dead_code)]
mod tuf_mothers;

#[tokio::test]
async fn filters_yanked_candidates_but_refuses_any_observed_revocation() {
    for status in ["yanked", "revoked"] {
        let (dir, policy) = registry_with_targets(|targets, _| {
            targets["records/loan-rules-1.0.0.json"]["custom"]["morphir"]["status"] =
                serde_json::json!(status);
        });
        let output = dir.path().join("new.lock");
        let result = mvp::resolve(ResolveRequest {
            policy: &policy,
            root: ReleaseId::new(
                PackagePath::parse("example.com/finance/eligibility").unwrap(),
                StableVersion::parse("1.2.0").unwrap(),
            ),
            registry: &dir.path().join("registry"),
            state: &dir.path().join("trust"),
            output: &output,
        })
        .await;
        if status == "yanked" {
            assert_eq!(result.unwrap().graph.nodes().len(), 1);
        } else {
            assert!(result.unwrap_err().to_string().contains("revocation"));
            assert!(!output.exists());
            assert!(dir.path().join("trust/operation").is_file());
        }
    }
}

#[tokio::test]
async fn does_not_select_yanked_provider_or_missing_root() {
    for missing_root in [false, true] {
        let (dir, policy) = registry_with_targets(|targets, _| {
            if !missing_root {
                targets["records/eligibility-1.2.0.json"]["custom"]["morphir"]["status"] =
                    serde_json::json!("yanked");
            }
        });
        let requested = if missing_root {
            ReleaseId::new(
                PackagePath::parse("example.com/finance/loan-rules").unwrap(),
                StableVersion::parse("9.9.9").unwrap(),
            )
        } else {
            root()
        };
        let output = dir.path().join("new.lock");
        let error = mvp::resolve(ResolveRequest {
            policy: &policy,
            root: requested,
            registry: &dir.path().join("registry"),
            state: &dir.path().join("trust"),
            output: &output,
        })
        .await
        .unwrap_err();
        assert!(
            error.to_string().starts_with("local Library operation")
                || error.to_string().starts_with("resolution rejected"),
            "{error}"
        );
        assert!(
            error.to_string().contains(if missing_root {
                "published root is unavailable"
            } else {
                "resolution rejected"
            }),
            "{error}"
        );
        assert!(!output.exists());
    }
}

#[tokio::test]
async fn rejects_duplicate_and_malformed_catalog_records_before_filtering() {
    for duplicate in [false, true] {
        let (dir, policy) = registry_with_targets(|targets, registry| {
            if duplicate {
                let target = targets["records/eligibility-1.2.0.json"].clone();
                let hash = target["hashes"]["sha256"].as_str().unwrap();
                fs::copy(
                    registry.join(format!("targets/records/{hash}.eligibility-1.2.0.json")),
                    registry.join(format!("targets/records/{hash}.duplicate.json")),
                )
                .unwrap();
                targets["records/duplicate.json"] = target;
                targets["records/duplicate.json"]["custom"]["morphir"]["status"] =
                    serde_json::json!("yanked");
            } else {
                targets["records/eligibility-1.2.0.json"]["custom"]["morphir"]["unexpected"] =
                    serde_json::json!(true);
            }
        });
        let output = dir.path().join("new.lock");
        let error = mvp::resolve(ResolveRequest {
            policy: &policy,
            root: root(),
            registry: &dir.path().join("registry"),
            state: &dir.path().join("trust"),
            output: &output,
        })
        .await
        .unwrap_err();
        assert!(
            error.to_string().contains(if duplicate {
                "duplicate catalog release"
            } else {
                "target custom"
            }),
            "{error}"
        );
        assert!(!output.exists());
    }
}
#[tokio::test]
async fn resolves_complete_lock_then_restores_it_and_repeats_deterministically() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("trust");
    let policy = fs::read_to_string(fixture().join("trust-policy.json"))
        .unwrap()
        .replace("previous-authorization", "fresh-metadata")
        .into_bytes();
    let bootstrap = fs::read(fixture().join("registry/metadata/1.root.json")).unwrap();
    mvp::initialize(InitializeRequest {
        policy: &policy,
        root: &bootstrap,
        state: &state,
    })
    .unwrap();
    let registry = fixture().join("registry");
    let mut locks = vec![];
    for name in ["first.lock", "second.lock"] {
        let output = dir.path().join(name);
        let report = mvp::resolve(ResolveRequest {
            policy: &policy,
            root: root(),
            registry: &registry,
            state: &state,
            output: &output,
        })
        .await
        .unwrap();
        assert_eq!(report.graph.root(), &root());
        assert_eq!(report.packages.len(), 2);
        let bytes = fs::read(output).unwrap();
        assert!(bytes.ends_with(b"\n"));
        // Dependency feature unification can enable serde_json/preserve_order.
        // The wire artifact must retain lexical object-key ordering regardless.
        let mut sorted: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        sorted.sort_all_objects();
        let mut lexical = serde_json::to_vec_pretty(&sorted).unwrap();
        lexical.push(b'\n');
        assert_eq!(
            bytes, lexical,
            "lock object order depends on dependency features"
        );
        let lock = decode_library_lock(&bytes).unwrap();
        assert_eq!(lock.graph(), &report.graph);
        assert_eq!(lock.acquisitions().len(), 2);
        assert_eq!(lock.evidence().len(), 6);
        assert_eq!(lock.registries()[0].id().as_str(), "local");
        locks.push(bytes);
    }
    assert_eq!(locks[0], locks[1]);
    let restored = dir.path().join("restored");
    let report = mvp::restore(RestoreRequest {
        policy: &policy,
        lock: &locks[0],
        registry: &registry,
        state: &state,
        output: &restored,
    })
    .await
    .unwrap();
    for package in report.packages {
        assert!(restored.join(package.directory).join("ir.json").is_file());
    }
}

#[tokio::test]
async fn resolve_preserves_unbounded_metadata_version_pins() {
    let (dir, policy) = registry_with_targets(|_, _| {});
    let registry = dir.path().join("registry");
    let version: serde_json::Value = serde_json::from_str("18446744073709551616").unwrap();
    let mut prior: Option<Vec<u8>> = None;
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
        if let Some(child) = child {
            let mut link = tuf_mothers::meta(prior.as_ref().unwrap(), 1);
            link["version"] = version.clone();
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
        prior = Some(bytes);
    }
    let output = dir.path().join("new.lock");
    mvp::resolve(ResolveRequest {
        policy: &policy,
        root: root(),
        registry: &registry,
        state: &dir.path().join("trust"),
        output: &output,
    })
    .await
    .unwrap();
    let bytes = fs::read(&output).unwrap();
    let lock = decode_library_lock(&bytes).unwrap();
    for role in ["timestamp", "snapshot", "targets"] {
        let evidence = lock
            .evidence()
            .iter()
            .find(|e| e.id().as_str() == role)
            .unwrap();
        assert_eq!(
            evidence.reference().path().as_str(),
            format!("metadata/{version}.{role}.json")
        );
    }
    mvp::restore(RestoreRequest {
        policy: &policy,
        lock: &bytes,
        registry: &registry,
        state: &dir.path().join("trust"),
        output: &dir.path().join("restored"),
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn content_and_publisher_failure_publish_no_lock_and_block_retry() {
    for fault in ["content", "inventory", "publisher"] {
        let (dir, policy) = registry_with_targets(|_, _| {});
        let registry = dir.path().join("registry");
        let mut policy: serde_json::Value = serde_json::from_slice(&policy).unwrap();
        let bundle = registry
            .join("bundles/5922bc8860f6cd008b9cda341be7f3a776ea332e63261392e94c17e19a647886");
        match fault {
            "content" => fs::write(bundle.join("ir.json"), b"corrupt").unwrap(),
            "inventory" => fs::write(bundle.join("extra.txt"), b"extra").unwrap(),
            "publisher" => {
                policy["publisherRules"][0]["publicKeys"] = serde_json::json!(["00".repeat(32)])
            }
            _ => unreachable!(),
        }
        let policy = serde_json::to_vec(&policy).unwrap();
        let output = dir.path().join("new.lock");
        let state = dir.path().join("trust");
        let error = mvp::resolve(ResolveRequest {
            policy: &policy,
            root: root(),
            registry: &registry,
            state: &state,
            output: &output,
        })
        .await
        .unwrap_err();
        let reason = match fault {
            "content" => "content digest mismatch",
            "inventory" => "bundle inventory",
            _ => "SignatureInvalid",
        };
        assert!(error.to_string().contains(reason), "{error}");
        assert!(!output.exists());
        assert!(state.join("operation").is_file());
        assert!(fs::read_dir(dir.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".morphir-resolve-")
        }));
        let error = mvp::resolve(ResolveRequest {
            policy: &policy,
            root: root(),
            registry: &registry,
            state: &state,
            output: &output,
        })
        .await
        .unwrap_err();
        assert!(
            error.to_string().contains("unresolved prior operation"),
            "{error}"
        );
    }
}

#[tokio::test]
async fn invalid_destination_and_root_authority_refuse_before_state_marker() {
    for fault in ["file", "directory", "missing-parent", "authority"] {
        let (dir, policy) = registry_with_targets(|_, _| {});
        let output = if fault == "missing-parent" {
            dir.path().join("missing/new.lock")
        } else {
            dir.path().join("new.lock")
        };
        if fault == "file" {
            fs::write(&output, b"keep me").unwrap();
        }
        if fault == "directory" {
            fs::create_dir(&output).unwrap();
        }
        let requested = if fault == "authority" {
            ReleaseId::new(
                PackagePath::parse("other.example/app").unwrap(),
                StableVersion::parse("1.0.0").unwrap(),
            )
        } else {
            root()
        };
        let error = mvp::resolve(ResolveRequest {
            policy: &policy,
            root: requested,
            registry: &dir.path().join("registry"),
            state: &dir.path().join("trust"),
            output: &output,
        })
        .await
        .unwrap_err();
        assert!(!dir.path().join("trust/operation").exists(), "{error}");
        if fault == "file" {
            assert_eq!(fs::read(&output).unwrap(), b"keep me");
        }
    }
}

#[tokio::test]
async fn concurrent_operation_and_failed_state_write_never_publish_lock() {
    let (dir, policy) = registry_with_targets(|_, _| {});
    let state = dir.path().join("trust");
    let output = dir.path().join("new.lock");
    let registry = dir.path().join("registry");
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(state.join("lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&held).unwrap();
    let error = mvp::resolve(ResolveRequest {
        policy: &policy,
        root: root(),
        registry: &registry,
        state: &state,
        output: &output,
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("locked by another"), "{error}");
    assert!(!state.join("operation").exists());
    drop(held);
    let db = rusqlite::Connection::open(state.join("trust.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_write BEFORE UPDATE ON state BEGIN SELECT RAISE(FAIL, 'injected write failure'); END;").unwrap();
    drop(db);
    let error = mvp::resolve(ResolveRequest {
        policy: &policy,
        root: root(),
        registry: &registry,
        state: &state,
        output: &output,
    })
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("injected write failure"),
        "{error}"
    );
    assert!(!output.exists());
    assert!(state.join("operation").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn refuses_dangling_lock_symlink_without_touching_its_target() {
    let (dir, policy) = registry_with_targets(|_, _| {});
    let output = dir.path().join("new.lock");
    let target = dir.path().join("absent");
    std::os::unix::fs::symlink(&target, &output).unwrap();
    assert!(
        mvp::resolve(ResolveRequest {
            policy: &policy,
            root: root(),
            registry: &dir.path().join("registry"),
            state: &dir.path().join("trust"),
            output: &output
        })
        .await
        .is_err()
    );
    assert_eq!(fs::read_link(&output).unwrap(), target);
    assert!(!target.exists());
    assert!(!dir.path().join("trust/operation").exists());
}

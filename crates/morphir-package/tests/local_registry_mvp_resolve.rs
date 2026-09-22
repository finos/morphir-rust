use morphir_package::{
    local_registry::{
        decode_library_lock,
        mvp::{self, InitializeRequest, ResolveRequest, RestoreRequest},
    },
    resolution::{PackagePath, ReleaseId, StableVersion},
};
use std::{fs, path::PathBuf};
#[path = "local_registry/tuf_mothers.rs"]
#[allow(dead_code)]
mod tuf_mothers;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/local_registry/mvp-fixture")
}

fn copy_tree(source: &std::path::Path, destination: &std::path::Path) {
    fs::create_dir(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Sign mutated repository inputs with an independent test signer. Publisher
/// envelopes and the expected Library graph stay frozen parent fixture bytes.
fn registry_with_targets(
    mutate: impl FnOnce(&mut serde_json::Value, &std::path::Path),
) -> (tempfile::TempDir, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("registry");
    copy_tree(&fixture().join("registry"), &registry);
    let mut targets: serde_json::Value =
        serde_json::from_slice(&fs::read(registry.join("metadata/1.targets.json")).unwrap())
            .unwrap();
    mutate(&mut targets["signed"]["targets"], &registry);
    let root = tuf_mothers::root(1, 42);
    let targets = tuf_mothers::sign(targets["signed"].clone(), &[(42, tuf_mothers::key(42))]);
    let snapshot = tuf_mothers::snapshot(&targets, 42);
    let timestamp = tuf_mothers::timestamp(&snapshot, 42);
    for (name, bytes) in [
        ("1.root.json", &root),
        ("1.targets.json", &targets),
        ("1.snapshot.json", &snapshot),
        ("1.timestamp.json", &timestamp),
        ("timestamp.json", &timestamp),
    ] {
        fs::write(registry.join("metadata").join(name), bytes).unwrap();
    }
    let mut policy: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture().join("trust-policy.json")).unwrap()).unwrap();
    policy["continuedUse"] = serde_json::json!("fresh-metadata");
    policy["repositories"][0]["bootstrapRoot"]["digest"] = serde_json::json!(format!(
        "sha256:{}",
        tuf_mothers::hex(<sha2::Sha256 as sha2::Digest>::digest(&root))
    ));
    let policy = serde_json::to_vec(&policy).unwrap();
    mvp::initialize(InitializeRequest {
        policy: &policy,
        root: &root,
        state: &dir.path().join("trust"),
    })
    .unwrap();
    (dir, policy)
}

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
fn root() -> ReleaseId {
    ReleaseId::new(
        PackagePath::parse("example.com/finance/loan-rules").unwrap(),
        StableVersion::parse("1.0.0").unwrap(),
    )
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

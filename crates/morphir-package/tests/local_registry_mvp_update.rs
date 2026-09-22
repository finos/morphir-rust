use morphir_package::{
    local_registry::mvp::{self, UpdateRequest},
    resolution::{PackagePath, UpdateTarget},
};
use std::fs;
#[path = "local_registry/resolve_mothers.rs"]
mod mothers;
#[path = "local_registry/tuf_mothers.rs"]
#[allow(dead_code)]
mod tuf_mothers;

#[tokio::test]
async fn update_reauthenticates_old_baseline_without_matching_historical_metadata() {
    let (dir, policy) = mothers::registry_with_targets(|targets, _| {
        targets["records/loan-rules-1.0.0.json"]["custom"]["morphir"]["status"] =
            serde_json::json!("yanked");
    });
    let old = fs::read(mothers::fixture().join("morphir.lock")).unwrap();
    let targets = [UpdateTarget::Eligible {
        package_path: PackagePath::parse("example.com/finance/eligibility").unwrap(),
    }];
    let output = dir.path().join("updated.lock");
    let report = mvp::update(UpdateRequest {
        policy: &policy,
        lock: &old,
        targets: &targets,
        registry: &dir.path().join("registry"),
        state: &dir.path().join("trust"),
        output: &output,
    })
    .await
    .unwrap();
    assert_eq!(report.graph.root(), &mothers::root());
    assert_eq!(report.graph.nodes().len(), 2);
    assert_eq!(
        old,
        fs::read(mothers::fixture().join("morphir.lock")).unwrap()
    );
    mvp::restore(mvp::RestoreRequest {
        policy: &policy,
        lock: &fs::read(output).unwrap(),
        registry: &dir.path().join("registry"),
        state: &dir.path().join("trust"),
        output: &dir.path().join("restored"),
    })
    .await
    .unwrap();
}
#[path = "local_registry/update_driver.rs"]
mod driver;
use driver::TestDriver;

#[tokio::test]
async fn eligible_and_exact_updates_match_independent_full_lock_goldens() {
    for (version, golden) in [
        (None, "update.lock.json"),
        (Some("1.2.0"), "exact-old.lock.json"),
    ] {
        let mut driver = TestDriver::provisioned(None);
        if let Some(version) = version {
            driver.exact(version);
        }
        driver.update().await;
        driver.assert_golden(golden);
        driver.restore().await;
    }
}
#[tokio::test]
async fn yanked_frozen_nodes_are_retained_and_restorable() {
    let mut driver = TestDriver::provisioned(Some("yanked-frozen"));
    driver.update().await;
    driver.assert_golden("yanked-frozen.lock.json");
    driver.restore().await;
}
#[tokio::test]
async fn scope_conflict_and_revocation_preserve_old_lock_and_block_restart() {
    for (variant, reason) in [
        ("scope-conflict", "ScopeConflict"),
        ("revoked-frozen", "revocation"),
    ] {
        let mut driver = TestDriver::provisioned(Some(variant));
        if variant == "scope-conflict" {
            driver.exact("1.4.0");
        }
        driver.update().await;
        driver.assert_refused(reason);
        driver.update().await;
        driver.assert_refused("unresolved prior operation");
    }
}
#[tokio::test]
async fn rejects_tampered_old_acquisitions_even_inside_update_closure() {
    for field in [
        "record-digest",
        "record-path",
        "source",
        "statement",
        "node-digest",
    ] {
        let mut driver = TestDriver::provisioned(None);
        let mut old: serde_json::Value = serde_json::from_slice(&driver.old).unwrap();
        match field {
            "record-digest" => {
                old["acquisitions"][0]["record"]["digest"] =
                    serde_json::json!(format!("sha256:{}", "0".repeat(64)))
            }
            "record-path" => {
                old["acquisitions"][0]["record"]["path"] = serde_json::json!("records/other.json")
            }
            "source" => {
                old["acquisitions"][0]["source"]["path"] = serde_json::json!("bundles/other")
            }
            "statement" => {
                let id = old["acquisitions"][0]["statement"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                let evidence = old["evidence"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|e| e["id"] == id)
                    .unwrap();
                evidence["digest"] = serde_json::json!(format!("sha256:{}", "0".repeat(64)));
            }
            "node-digest" => {
                old["graph"]["nodes"][1]["manifestDigest"] =
                    serde_json::json!(format!("sha256:{}", "0".repeat(64)))
            }
            _ => unreachable!(),
        }
        driver.old = serde_json::to_vec(&old).unwrap();
        driver.update().await;
        driver.assert_refused(if field == "node-digest" {
            "InvalidLock"
        } else {
            "old record acquisition mismatch"
        });
    }
}
#[tokio::test]
async fn rejects_new_yanked_exact_target_and_invalid_target_scope() {
    let mut driver = TestDriver::provisioned(None);
    driver.exact("1.9.0");
    driver.update().await;
    driver.assert_refused("Unsatisfiable");
    for path in [
        "example.com/finance/loan-rules",
        "example.com/finance/unknown",
    ] {
        let mut driver = TestDriver::provisioned(None);
        driver.targets(vec![UpdateTarget::Eligible {
            package_path: PackagePath::parse(path).unwrap(),
        }]);
        driver.update().await;
        driver.assert_refused("InvalidInput");
    }
}

#[tokio::test]
async fn update_failures_leave_no_output_and_preserve_unresolved_state() {
    for fault in ["content", "publisher", "state-write"] {
        let mut driver = TestDriver::provisioned(None);
        if fault == "content" {
            let old: serde_json::Value = serde_json::from_slice(&driver.old).unwrap();
            let source = old["acquisitions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["release"]["packagePath"] == "example.com/finance/sibling")
                .unwrap()["source"]["path"]
                .as_str()
                .unwrap();
            fs::write(
                driver.path("registry").join(source).join("ir.json"),
                b"corrupt",
            )
            .unwrap();
        } else if fault == "publisher" {
            let mut policy: serde_json::Value = serde_json::from_slice(&driver.policy).unwrap();
            policy["publisherRules"][0]["publicKeys"] = serde_json::json!(["00".repeat(32)]);
            driver.policy = serde_json::to_vec(&policy).unwrap();
        } else {
            let db = rusqlite::Connection::open(driver.path("trust/trust.sqlite")).unwrap();
            db.execute_batch("CREATE TRIGGER fail_write BEFORE UPDATE ON state BEGIN SELECT RAISE(FAIL, 'injected write failure'); END;").unwrap();
        }
        driver.update().await;
        driver.assert_refused(match fault {
            "content" => "content digest mismatch",
            "publisher" => "SignatureInvalid",
            _ => "injected write failure",
        });
        driver.update().await;
        driver.assert_refused("unresolved prior operation");
    }
}
#[tokio::test]
async fn occupied_destination_and_concurrent_update_refuse_without_starting_operation() {
    let mut driver = TestDriver::provisioned(None);
    fs::write(driver.path("updated.lock"), b"keep me").unwrap();
    driver.update().await;
    assert!(
        driver
            .result
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("destination already exists")
    );
    assert_eq!(fs::read(driver.path("updated.lock")).unwrap(), b"keep me");
    assert!(!driver.path("trust/operation").exists());
    fs::remove_file(driver.path("updated.lock")).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(driver.path("trust/lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&held).unwrap();
    driver.update().await;
    driver.assert_refused("locked by another");
    assert!(!driver.path("trust/operation").exists());
}

#[tokio::test]
async fn yanked_root_is_frozen_and_restorable() {
    let mut driver = TestDriver::provisioned(Some("yanked-root"));
    driver.update().await;
    driver.assert_golden("yanked-root.lock.json");
    driver.restore().await;
}

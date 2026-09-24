use super::*;
use morphir_common::home::MorphirHome;
use morphir_distribution::{
    ExtensionId, ExtensionInstaller, LocalIndex, Platform, Selection, Sha256Digest,
    activate_installed, list_installed, read_extension_lock,
};

struct ClaimsInstall {
    _root: tempfile::TempDir,
    home: MorphirHome,
    id: ExtensionId,
}

impl ClaimsInstall {
    fn a_frontend(draft: &str) -> Self {
        let mut release = an_old_release();
        for key in ["mepVersions", "capabilities", "backend"] {
            release.as_object_mut().unwrap().remove(key);
        }
        release["schemaVersion"] = json!(format!("2.0.0-draft.{draft}"));
        let mut claims = a_claims();
        claims["extension"]["types"] = json!(["frontend"]);
        claims["capabilities"] = json!({"frontend": {
            "languages": [{"id": "elm", "fileExtensions": [".elm"]}],
            "irVersions": ["3"], "compile": true,
            "multiDocument": true, "fragments": true
        }});
        claims["futureDocumentMember"] = json!({"preserved": true});
        if draft == "1" {
            claims.as_object_mut().unwrap().remove("claimsVersion");
            claims["statementVersion"] = json!("0.1.0-draft.1");
            release["artifacts"][0]["statement"] = claims;
        } else {
            release["artifacts"][0]["claims"] = claims;
        }
        Self::from_release(release)
    }

    fn from_release(mut release: Value) -> Self {
        let root = tempfile::tempdir().unwrap();
        let index = root.path().join("index");
        std::fs::create_dir_all(index.join("extensions")).unwrap();
        std::fs::create_dir_all(index.join("artifacts")).unwrap();
        std::fs::write(index.join("artifacts/sample.wasm"), b"wasm").unwrap();
        release["artifacts"][0]["sha256"] = json!(Sha256Digest::of_bytes(b"wasm"));
        std::fs::write(index.join("extensions/sample.jsonl"), release.to_string()).unwrap();
        let id = ExtensionId::parse("sample").unwrap();
        let selected = LocalIndex::open(&index)
            .unwrap()
            .resolve(
                &id,
                Selection::Exact(semver::Version::new(1, 0, 0)),
                &Platform::current(),
                &"0.4.0".parse().unwrap(),
            )
            .unwrap();
        let home =
            MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
        ExtensionInstaller::new(&home)
            .install(selected, &"0.4.0".parse().unwrap())
            .unwrap();
        Self {
            _root: root,
            home,
            id,
        }
    }

    fn lock_path(&self) -> std::path::PathBuf {
        self.home.extensions_locks_dir().join("sample.json")
    }

    fn assert_activates_with_claim_flags(&self) {
        assert_eq!(list_installed(&self.home).unwrap().len(), 1);
        let artifact = activate_installed(&self.home, &self.id).unwrap();
        let frontend = artifact.extension_capabilities().frontend.unwrap();
        assert!(frontend.multi_document);
        assert!(frontend.fragments);
    }
}

fn read_json(path: &std::path::Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn edit_json(path: &std::path::Path, edit: impl FnOnce(&mut Value)) {
    let mut value = read_json(path);
    edit(&mut value);
    std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

#[test]
fn claims_lock_round_trips_complete_supplied_claims_in_both_drafts() {
    for draft in ["1", "2"] {
        let install = ClaimsInstall::a_frontend(draft);
        let catalog = read_json(&install.home.extensions_catalog_file());
        let lock = read_json(&install.lock_path());
        assert_eq!(lock["selection"]["kind"], "exact");
        assert_eq!(lock["index"]["claims"], catalog["extensions"][0]["claims"]);
        let reloaded = read_extension_lock(&install.home, &install.id).unwrap();
        assert_eq!(serde_json::to_value(reloaded).unwrap(), lock);
        install.assert_activates_with_claim_flags();
    }
}

#[test]
fn claims_lock_refuses_each_edited_frontend_flag() {
    for draft in ["1", "2"] {
        for flag in ["multiDocument", "fragments"] {
            let install = ClaimsInstall::a_frontend(draft);
            edit_json(&install.home.extensions_catalog_file(), |catalog| {
                catalog["extensions"][0]["claims"]["capabilities"]["frontend"][flag] = json!(false);
            });
            for error in [
                list_installed(&install.home).unwrap_err(),
                activate_installed(&install.home, &install.id).unwrap_err(),
            ] {
                let message = error.to_string();
                assert!(
                    message.contains(&format!("claims.capabilities.frontend.{flag}")),
                    "{message}"
                );
            }
        }
    }
}

#[test]
fn claims_lock_refuses_removed_claims_and_unknown_member_edits() {
    for remove_claims in [false, true] {
        let install = ClaimsInstall::a_frontend("2");
        edit_json(&install.home.extensions_catalog_file(), |catalog| {
            if remove_claims {
                catalog["extensions"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("claims");
            } else {
                catalog["extensions"][0]["claims"]["futureDocumentMember"]["preserved"] =
                    json!(false);
            }
        });
        let error = activate_installed(&install.home, &install.id)
            .unwrap_err()
            .to_string();
        let path = if remove_claims {
            "claims"
        } else {
            "claims.futureDocumentMember.preserved"
        };
        assert!(error.contains(path), "{error}");
    }
}

#[test]
fn claims_lock_without_pin_keeps_beta_seven_activation_behavior() {
    for draft in ["1", "2"] {
        let install = ClaimsInstall::a_frontend(draft);
        edit_json(&install.lock_path(), |lock| {
            lock["index"].as_object_mut().unwrap().remove("claims");
        });
        install.assert_activates_with_claim_flags();
    }
}

#[test]
fn claims_lock_is_readable_by_released_beta_readers() {
    let install = ClaimsInstall::a_frontend("2");
    let lock = read_json(&install.lock_path());
    assert!(lock["index"].get("claims").is_some());
    let old: released_reader::ExtensionLock = serde_json::from_value(lock.clone()).unwrap();
    let mut old_shape = lock;
    old_shape["index"].as_object_mut().unwrap().remove("claims");
    assert_eq!(serde_json::to_value(old).unwrap(), old_shape);
}

#[test]
fn legacy_install_writes_exactly_the_released_lock_bytes() {
    let install = ClaimsInstall::from_release(an_old_release());
    let bytes = std::fs::read(install.lock_path()).unwrap();
    let old: released_reader::ExtensionLock = serde_json::from_slice(&bytes).unwrap();
    let mut old_bytes = serde_json::to_vec_pretty(&old).unwrap();
    old_bytes.push(b'\n');
    assert_eq!(bytes, old_bytes);
    assert!(
        read_json(&install.lock_path())["index"]
            .get("claims")
            .is_none()
    );
    activate_installed(&install.home, &install.id).unwrap();
}

#[test]
fn claims_lock_ignores_json_object_order_and_optional_index_members() {
    let install = ClaimsInstall::a_frontend("2");
    edit_json(&install.lock_path(), |lock| {
        let claims = lock["index"]["claims"].as_object_mut().unwrap();
        let old = std::mem::take(claims);
        claims.extend(old.into_iter().rev());
        lock["index"]["futureLockMetadata"] = json!({"ignored": true});
    });
    install.assert_activates_with_claim_flags();
}

#[test]
fn version_one_lock_bytes_are_unchanged() {
    let bytes = include_bytes!("../fixtures/frontend-lock-v1.json");
    let lock: morphir_distribution::ExtensionLock = serde_json::from_slice(bytes).unwrap();
    let mut written = serde_json::to_vec_pretty(&lock).unwrap();
    written.push(b'\n');
    assert_eq!(written, bytes);
}

#[path = "released_lock_reader.rs"]
mod released_reader;

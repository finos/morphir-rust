use morphir_distribution::{InstalledExtension, ReleaseRecord};
use serde_json::{Value, json};

fn an_old_release() -> Value {
    json!({"schemaVersion":"1.0", "id":"sample", "name":"Sample", "version":"1.0.0",
        "mepVersions":["0.1"], "capabilities":["backend"],
        "backend":{"targets":["text"],"irVersions":["3"],"generate":true},
        "artifacts":[{"runtime":"wasm","filename":"sample.wasm","sha256":"a".repeat(64),
            "source":{"kind":"local-file","path":"artifacts/sample.wasm"}}]})
}

fn an_old_installed_record() -> Value {
    json!({"extensionId":"sample","name":"Sample","version":"1.0.0","runtime":"wasm",
        "platform":null,"args":[],"digest":"a".repeat(64),"storePath":"extensions/sample.wasm",
        "capabilities":["backend"],"mepVersions":["0.1"],
        "backend":{"targets":["text"],"irVersions":["3"],"generate":true},"executable":false,
        "index":{"kind":"local-directory","identity":"/tmp/index","revision":"b".repeat(64)}})
}

fn a_claims() -> Value {
    json!({"claimsVersion":"0.1.0-draft.2","protocolVersions":["0.1"],
        "extension":{"id":"sample","name":"Sample","version":"1.0.0","types":["backend"]},
        "capabilities":{"backend":{"targets":["text"],"irVersions":["3"],"generate":true,"future":42}}})
}

#[test]
fn index_ignores_optional_members_at_every_extension_level() {
    let mut value = an_old_release();
    value["future"] = json!(true);
    value["backend"]["future"] = json!(true);
    value["artifacts"][0]["future"] = json!(true);
    value["artifacts"][0]["source"]["future"] = json!(true);
    assert!(serde_json::from_value::<ReleaseRecord>(value).is_ok());
}

#[test]
fn installed_ignores_optional_members() {
    let mut value = an_old_installed_record();
    value["future"] = json!(true);
    value["index"]["future"] = json!(true);
    assert!(serde_json::from_value::<InstalledExtension>(value).is_ok());
}

#[test]
fn index_checks_critical_paths_by_name() {
    let mut value = an_old_release();
    value["critical"] = json!(["backend.targets"]);
    assert!(serde_json::from_value::<ReleaseRecord>(value.clone()).is_ok());
    value["critical"] = json!(["backend.future"]);
    let error = serde_json::from_value::<ReleaseRecord>(value).unwrap_err();
    assert!(error.to_string().contains("backend.future"));
}

#[test]
fn installed_checks_critical_paths_by_name() {
    let mut value = an_old_installed_record();
    value["critical"] = json!(["backend.targets"]);
    assert!(serde_json::from_value::<InstalledExtension>(value.clone()).is_ok());
    value["critical"] = json!(["future"]);
    let error = serde_json::from_value::<InstalledExtension>(value).unwrap_err();
    assert!(error.to_string().contains("future"));
}

#[test]
fn index_accepts_supported_schema_versions_and_missing_version() {
    for version in [
        None,
        Some("1.0"),
        Some("1.0.0"),
        Some("1.9.27"),
        Some("2.0.0-draft.1"),
        Some("2.0.0-draft.2"),
    ] {
        let mut value = an_old_release();
        value.as_object_mut().unwrap().remove("schemaVersion");
        if let Some(version) = version {
            value["schemaVersion"] = json!(version);
        }
        assert!(
            serde_json::from_value::<ReleaseRecord>(value).is_ok(),
            "{version:?}"
        );
    }
}

#[test]
fn index_refuses_other_schema_versions() {
    for version in [
        "0.9.0",
        "2.0.0",
        "3.0.0",
        "1.0.0-rc.1",
        "2.0.0-draft.3",
        "2.1.0-draft.1",
        "banana",
    ] {
        let mut value = an_old_release();
        value["schemaVersion"] = json!(version);
        assert!(
            serde_json::from_value::<ReleaseRecord>(value).is_err(),
            "{version}"
        );
    }
}

#[test]
fn index_retains_each_artifacts_claims() {
    let mut value = an_old_release();
    value["artifacts"][0]["claims"] = a_claims();
    value["artifacts"][0]["claimCheck"] = json!("probed");
    let record: ReleaseRecord = serde_json::from_value(value.clone()).unwrap();
    let written = serde_json::to_value(record).unwrap();
    assert_eq!(
        written["artifacts"][0]["claims"],
        value["artifacts"][0]["claims"]
    );
    assert_eq!(written["artifacts"][0]["claimCheck"], "probed");
}

#[test]
fn installed_retains_claims_and_provenance() {
    let mut value = an_old_installed_record();
    value["claims"] = a_claims();
    value["claimCheck"] = json!("unchecked");
    let record: InstalledExtension = serde_json::from_value(value.clone()).unwrap();
    let written = serde_json::to_value(record).unwrap();
    assert_eq!(written["claims"], value["claims"]);
    assert_eq!(written["claimCheck"], "unchecked");
}

#[test]
fn index_checks_host_comparators_and_requires_critical() {
    let mut value = an_old_release();
    value["requires"] = json!({"host":[">=0.1.0","<1.0.0"]});
    value["critical"] = json!(["requires.host"]);
    assert!(serde_json::from_value::<ReleaseRecord>(value.clone()).is_ok());
    value["requires"]["host"] = json!([">=999.0.0"]);
    assert!(serde_json::from_value::<ReleaseRecord>(value.clone()).is_ok());
    value["requires"]["host"] = json!([">=0.1.0, <1.0.0"]);
    assert!(serde_json::from_value::<ReleaseRecord>(value.clone()).is_err());
    value["requires"]["host"] = json!([">=0.1.0"]);
    value.as_object_mut().unwrap().remove("critical");
    assert!(
        serde_json::from_value::<ReleaseRecord>(value)
            .unwrap_err()
            .to_string()
            .contains("requires.host")
    );
}

#[test]
fn old_flat_records_convert_to_declared_without_changing_writers() {
    use morphir_distribution::ClaimCheck;
    let release: ReleaseRecord = serde_json::from_value(an_old_release()).unwrap();
    let claims = release.artifacts()[0].claims().unwrap();
    assert_eq!(claims.extension.id, "sample");
    assert_eq!(claims.capabilities["backend"]["targets"], json!(["text"]));
    assert_eq!(release.artifacts()[0].claim_check(), ClaimCheck::Unchecked);
    assert!(
        serde_json::to_value(release).unwrap()["artifacts"][0]
            .get("claims")
            .is_none()
    );
    let installed: InstalledExtension = serde_json::from_value(an_old_installed_record()).unwrap();
    assert_eq!(installed.claims().extension.id, "sample");
    assert_eq!(installed.claim_check(), ClaimCheck::Unchecked);
    assert!(
        serde_json::to_value(installed)
            .unwrap()
            .get("claims")
            .is_none()
    );
}

fn read_catalog(
    value: Value,
) -> Result<morphir_distribution::InstalledCatalog, morphir_distribution::DistributionError> {
    let root = tempfile::tempdir().unwrap();
    let home = morphir_common::home::MorphirHome::resolve_from(
        Some(root.path().join("home").as_os_str()),
        None,
    )
    .unwrap();
    let path = home.extensions_catalog_file();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    morphir_distribution::InstalledCatalog::load(&home)
}

#[test]
fn catalog_ignores_optional_members_and_checks_critical_by_name() {
    let mut value = json!({"extensions": [an_old_installed_record()], "future":true});
    assert!(read_catalog(value.clone()).is_ok());
    value["critical"] = json!(["extensions.backend.targets"]);
    assert!(read_catalog(value.clone()).is_ok());
    value["critical"] = json!(["extensions.future"]);
    assert!(
        read_catalog(value)
            .unwrap_err()
            .to_string()
            .contains("extensions.future")
    );
}

#[test]
fn catalog_accepts_supported_versions_and_refuses_others() {
    for version in ["1.0", "1.0.0", "1.42.3", "2.0.0-draft.1", "2.0.0-draft.2"] {
        assert!(
            read_catalog(json!({"schemaVersion":version,"extensions":[]})).is_ok(),
            "{version}"
        );
    }
    for version in [
        "0.9.0",
        "2.0.0",
        "3.0.0",
        "1.0.0-rc.1",
        "2.0.0-draft.3",
        "2.1.0-draft.1",
        "banana",
    ] {
        assert!(
            read_catalog(json!({"schemaVersion":version,"extensions":[]})).is_err(),
            "{version}"
        );
    }
}

#[test]
fn claims_only_records_need_no_flat_capability_metadata() {
    let mut release = an_old_release();
    for key in ["mepVersions", "capabilities", "backend"] {
        release.as_object_mut().unwrap().remove(key);
    }
    release["schemaVersion"] = json!("2.0.0-draft.2");
    release["artifacts"][0]["claims"] = a_claims();
    let parsed = serde_json::from_value::<ReleaseRecord>(release).unwrap();
    assert_eq!(
        parsed.artifacts()[0].claims().unwrap().capabilities["backend"]["future"],
        42
    );
    let mut installed = an_old_installed_record();
    for key in ["mepVersions", "capabilities", "backend"] {
        installed.as_object_mut().unwrap().remove(key);
    }
    installed["claims"] = a_claims();
    let parsed = serde_json::from_value::<InstalledExtension>(installed).unwrap();
    assert_eq!(parsed.claims().capabilities["backend"]["future"], 42);
}

#[test]
fn supplied_claim_sets_parse_without_comparing_host_requirements() {
    let mut value = an_old_release();
    let mut claims = a_claims();
    claims["requires"] = json!({"host":[">=999.0.0"]});
    claims["critical"] = json!(["requires.host"]);
    value["artifacts"][0]["claims"] = claims;
    assert!(serde_json::from_value::<ReleaseRecord>(value).is_ok());
}

#[test]
fn installing_a_claims_only_index_record_keeps_the_selected_claims() {
    use morphir_distribution::{
        ExtensionId, ExtensionInstaller, LocalIndex, Platform, Selection, Sha256Digest,
    };
    let root = tempfile::tempdir().unwrap();
    let index = root.path().join("index");
    std::fs::create_dir_all(index.join("extensions")).unwrap();
    std::fs::create_dir_all(index.join("artifacts")).unwrap();
    std::fs::write(index.join("artifacts/sample.wasm"), b"wasm").unwrap();
    let mut value = an_old_release();
    for key in ["mepVersions", "capabilities", "backend"] {
        value.as_object_mut().unwrap().remove(key);
    }
    value["schemaVersion"] = json!("2.0.0-draft.2");
    value["artifacts"][0]["claims"] = a_claims();
    value["artifacts"][0]["sha256"] = json!(Sha256Digest::of_bytes(b"wasm"));
    std::fs::write(index.join("extensions/sample.jsonl"), value.to_string()).unwrap();
    let selected = LocalIndex::open(&index)
        .unwrap()
        .resolve(
            &ExtensionId::parse("sample").unwrap(),
            Selection::Exact(semver::Version::new(1, 0, 0)),
            &Platform::current(),
            &"0.4.0".parse().unwrap(),
        )
        .unwrap();
    let home = morphir_common::home::MorphirHome::resolve_from(
        Some(root.path().join("home").as_os_str()),
        None,
    )
    .unwrap();
    let installed = ExtensionInstaller::new(&home)
        .install(selected, &"0.4.0".parse().unwrap())
        .unwrap();
    let catalog: Value =
        serde_json::from_slice(&std::fs::read(home.extensions_catalog_file()).unwrap()).unwrap();
    assert_eq!(catalog["schemaVersion"], "2.0.0-draft.2");
    assert!(catalog["extensions"][0].get("statement").is_none());
    assert_eq!(
        catalog["extensions"][0]["claims"]["claimsVersion"],
        "0.1.0-draft.2"
    );
    assert_eq!(installed.claims().capabilities["backend"]["future"], 42);
    assert_eq!(
        serde_json::to_value(installed).unwrap()["claims"],
        a_claims()
    );
}

#[test]
fn critical_paths_cannot_claim_unimplemented_flat_members() {
    let mut value = an_old_release();
    value["workspaceDiscovery"] = json!(true);
    value["critical"] = json!(["workspaceDiscovery"]);
    assert!(
        serde_json::from_value::<ReleaseRecord>(value)
            .unwrap_err()
            .to_string()
            .contains("workspaceDiscovery")
    );
    let mut value = an_old_installed_record();
    value["workspaceDiscovery"] = json!(true);
    value["critical"] = json!(["workspaceDiscovery"]);
    assert!(
        serde_json::from_value::<InstalledExtension>(value)
            .unwrap_err()
            .to_string()
            .contains("workspaceDiscovery")
    );
}

#[test]
fn critical_claims_paths_must_belong_to_the_current_format() {
    let mut value = an_old_release();
    value["claims"] = a_claims();
    value["critical"] = json!(["claims.requires.host"]);
    assert!(
        serde_json::from_value::<ReleaseRecord>(value)
            .unwrap_err()
            .to_string()
            .contains("claims.requires.host")
    );
    let mut value = an_old_release();
    value["artifacts"][0]["critical"] = json!(["extensions.claims.requires.host"]);
    assert!(
        serde_json::from_value::<ReleaseRecord>(value)
            .unwrap_err()
            .to_string()
            .contains("extensions.claims.requires.host")
    );
}

#[path = "extension_readers/compatibility.rs"]
mod compatibility;

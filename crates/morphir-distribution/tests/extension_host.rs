use morphir_common::home::MorphirHome;
use morphir_distribution::{
    ExtensionHistory, ExtensionId, ExtensionInstaller, InstalledCatalog, InstalledExtension,
    LocalIndex, Platform, ReleaseRecord, Selection, Sha256Digest, resolve,
};
use semver::Version;
use serde_json::{Value, json};

fn a_release_with_requirements(location: &str, comparators: Option<Value>) -> Value {
    let mut value = json!({
        "id": "sample", "name": "Sample", "version": "1.0.0",
        "artifacts": [{
            "runtime": "wasm", "filename": "sample.wasm",
            "sha256": Sha256Digest::of_bytes(b"wasm"),
            "source": {"kind": "local-file", "path": "artifacts/sample.wasm"},
            "statement": {
                "statementVersion": "0.1.0-draft.1", "protocolVersions": ["0.1"],
                "extension": {
                    "id": "sample", "name": "Sample", "version": "1.0.0", "types": ["backend"]
                },
                "capabilities": {
                    "backend": {"targets": ["text"], "irVersions": ["3"], "generate": true}
                }
            }
        }]
    });
    if let Some(comparators) = comparators {
        let target = match location {
            "release" => &mut value,
            "statement" => &mut value["artifacts"][0]["statement"],
            _ => unreachable!(),
        };
        target["requires"] = json!({"host": comparators});
        target["critical"] = json!(["requires.host"]);
    }
    value
}

fn an_index(root: &std::path::Path, release: &Value) -> LocalIndex {
    let index = root.join("index");
    std::fs::create_dir_all(index.join("extensions")).unwrap();
    std::fs::create_dir_all(index.join("artifacts")).unwrap();
    std::fs::write(index.join("artifacts/sample.wasm"), b"wasm").unwrap();
    std::fs::write(index.join("extensions/sample.jsonl"), release.to_string()).unwrap();
    LocalIndex::open(index).unwrap()
}

#[test]
fn resolve_install_and_installed_checks_use_the_callers_host() {
    for location in ["release", "statement"] {
        for (comparators, host, compatible_host, met) in [
            (Some(json!([">=0.4.0"])), "0.4.0", "0.4.0", true),
            (Some(json!(["<0.3.0"])), "0.4.0", "0.2.0", false),
            (
                Some(json!([">=0.4.0-alpha.7", "<0.5.0"])),
                "0.4.0-beta.4",
                "0.4.0-beta.4",
                true,
            ),
            (Some(json!([">=0.4.0"])), "0.4.0-beta.4", "0.4.0", false),
            (None, "0.4.0-beta.4", "0.4.0-beta.4", true),
        ] {
            let value = a_release_with_requirements(location, comparators.clone());
            let host: Version = host.parse().unwrap();
            let compatible_host: Version = compatible_host.parse().unwrap();
            let root = tempfile::tempdir().unwrap();
            let index = an_index(root.path(), &value);
            let id = ExtensionId::parse("sample").unwrap();
            let selection = Selection::Exact(Version::new(1, 0, 0));
            let platform = Platform::current();
            let history = ExtensionHistory::parse_jsonl(value.to_string().as_bytes()).unwrap();
            let resolved = resolve(&history, &selection, &platform, &host);
            assert_eq!(resolved.is_ok(), met, "{location}: {comparators:?}, {host}");
            let local = index.resolve(&id, selection.clone(), &platform, &host);
            assert_eq!(local.is_ok(), met, "{location}: {comparators:?}, {host}");
            if let Err(error) = local {
                assert_host_error(error, &host, comparators.as_ref().unwrap());
            }

            // Resolve on a compatible host, then install on a different host.
            let resolved = resolve(&history, &selection, &platform, &compatible_host).unwrap();
            assert_eq!(resolved.check_host(&host).is_ok(), met);
            let selected = index
                .resolve(&id, selection, &platform, &compatible_host)
                .unwrap();
            assert_eq!(selected.check_host(&host).is_ok(), met);
            let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None)
                .unwrap();
            let installer = ExtensionInstaller::new(&home);
            let installed = installer.install(selected.clone(), &host);
            assert_eq!(installed.is_ok(), met);
            if let Err(error) = installed {
                assert_host_error(error, &host, comparators.as_ref().unwrap());
                assert!(!home.extensions_catalog_file().exists());
                assert!(!home.extensions_locks_dir().exists());
                assert!(!home.root().exists());
            }

            let installed = installer.install(selected, &compatible_host).unwrap();
            assert_eq!(installed.check_host(&host).is_ok(), met);
            let wire = serde_json::to_value(installed).unwrap();
            // Reading installed metadata never performs a version comparison.
            let reread: InstalledExtension = serde_json::from_value(wire).unwrap();
            assert_eq!(reread.check_host(&host).is_ok(), met);
            let catalog = InstalledCatalog::load(&home).unwrap();
            assert_eq!(catalog.get(&id).unwrap().check_host(&host).is_ok(), met);
        }
    }
}

fn assert_host_error(error: impl std::fmt::Display, host: &Version, comparators: &Value) {
    let message = error.to_string();
    assert!(message.contains(&host.to_string()), "{message}");
    assert!(message.contains("requires.host"), "{message}");
    for comparator in comparators.as_array().unwrap() {
        assert!(message.contains(comparator.as_str().unwrap()), "{message}");
    }
}

#[test]
fn requirement_readers_reject_invalid_comparators_and_missing_critical() {
    for location in ["release", "statement"] {
        for comparators in [
            json!(">=0.4.0"),
            json!([">=0.4.0, <1.0.0"]),
            json!(["banana"]),
            json!([42]),
            Value::Null,
        ] {
            let value = a_release_with_requirements(location, Some(comparators));
            assert!(serde_json::from_value::<ReleaseRecord>(value).is_err());
        }
        let mut value = a_release_with_requirements(location, Some(json!([">=0.4.0"])));
        let target = if location == "release" {
            &mut value
        } else {
            &mut value["artifacts"][0]["statement"]
        };
        target.as_object_mut().unwrap().remove("critical");
        let error = serde_json::from_value::<ReleaseRecord>(value).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("requires.host must be listed in critical")
        );
    }
}

#[test]
fn resolve_checks_only_the_selected_artifacts_statement() {
    let mut value = a_release_with_requirements("statement", Some(json!([">=0.4.0"])));
    let mut other = value["artifacts"][0].clone();
    other["runtime"] = json!("process");
    other["platform"] = json!({"os": "other", "arch": "other"});
    other["executable"] = json!(true);
    other["statement"]["requires"]["host"] = json!([">=999.0.0"]);
    value["artifacts"].as_array_mut().unwrap().push(other);
    let history = ExtensionHistory::parse_jsonl(value.to_string().as_bytes()).unwrap();
    resolve(
        &history,
        &Selection::Exact(Version::new(1, 0, 0)),
        &Platform::current(),
        &Version::new(0, 4, 0),
    )
    .unwrap();
}

#[test]
fn release_and_statement_requirements_are_both_enforced() {
    let mut value = a_release_with_requirements("release", Some(json!([">=0.4.0"])));
    value["artifacts"][0]["statement"]["requires"] = json!({"host": ["<0.5.0"]});
    value["artifacts"][0]["statement"]["critical"] = json!(["requires.host"]);
    let history = ExtensionHistory::parse_jsonl(value.to_string().as_bytes()).unwrap();
    for (host, met) in [("0.3.0", false), ("0.4.0", true), ("0.5.0", false)] {
        assert_eq!(
            resolve(
                &history,
                &Selection::Exact(Version::new(1, 0, 0)),
                &Platform::current(),
                &host.parse().unwrap(),
            )
            .is_ok(),
            met
        );
    }
}

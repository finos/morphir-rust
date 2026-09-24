use morphir_distribution::{LocalExtensionRepository, Sha256Digest};
use morphir_extension_sdk::protocol::MEP_VERSION;
use serde_json::{Value, json};
use std::{fs, path::Path};

fn claims() -> Value {
    json!({"claimsVersion": "0.1.0-draft.2", "protocolVersions": [MEP_VERSION],
        "extension": {"id": "example", "name": "Example", "version": "1.0.0", "types": ["backend"]},
        "capabilities": {"backend": {"targets": ["sql"], "irVersions": ["3"], "generate": true}}})
}

fn bundle(root: &Path, claim_sets: &[Value]) -> std::path::PathBuf {
    let bundle = root.join("bundle");
    fs::create_dir(&bundle).unwrap();
    let artifacts: Vec<_> = claim_sets.iter().enumerate().map(|(index, claims)| {
        let filename = format!("guest-{index}");
        let digest = Sha256Digest::of_bytes(b"guest");
        fs::write(bundle.join(&filename), b"guest").unwrap();
        fs::write(bundle.join(format!("{filename}.sha256")), format!("{digest}  {filename}\n")).unwrap();
        // These targets are foreign on the supported Unix test hosts.
        json!({"runtime": "process", "platform": if index == 0 { "x86_64-pc-windows-msvc" } else { "aarch64-pc-windows-msvc" },
            "filename": filename, "sha256": digest, "claims": claims, "claimCheck": "probed"})
    }).collect();
    fs::write(bundle.join("release.json"), serde_json::to_vec(&json!({
        "schemaVersion": "2.0.0-draft.2", "extensionId": "example", "shortId": "example", "version": "1.0.0",
        "platformDifferences": "none", "artifacts": artifacts
    })).unwrap()).unwrap();
    bundle
}

fn edit(bundle: &Path, f: impl FnOnce(&mut Value)) {
    let path = bundle.join("release.json");
    let mut value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    f(&mut value);
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

#[test]
#[cfg(unix)]
fn foreign_process_artifacts_keep_claim_sets_and_are_declared() {
    let temp = tempfile::tempdir().unwrap();
    let mut declared = claims();
    declared["future"] = json!({"retained": true});
    let bundle = bundle(temp.path(), &[declared.clone(), declared.clone()]);
    let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
    let published = repository.publish(&bundle).unwrap();
    let wire = serde_json::to_value(published.release()).unwrap();
    assert_eq!(wire["schemaVersion"], "2.0.0-draft.2");
    for artifact in wire["artifacts"].as_array().unwrap() {
        assert_eq!(artifact["claims"], declared);
        assert_eq!(artifact["claimCheck"], "unchecked");
    }
    assert!(repository.root().join("artifacts/guest-1").exists());
    assert_eq!(
        repository.publish(&bundle).unwrap().status(),
        morphir_distribution::PublicationStatus::AlreadyPresent
    );
}

#[test]
#[cfg(unix)]
fn platform_differences_require_explicit_declaration() {
    let temp = tempfile::tempdir().unwrap();
    let mut other = claims();
    other["capabilities"]["backend"]["generate"] = json!(false);
    let bundle = bundle(temp.path(), &[claims(), other]);
    let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
    let error = repository.publish(&bundle).unwrap_err().to_string();
    assert!(
        error.contains("platformDifferences") && error.contains("capabilities.backend.generate"),
        "{error}"
    );
    edit(&bundle, |value| {
        value.as_object_mut().unwrap().remove("platformDifferences");
    });
    assert!(
        repository
            .publish(&bundle)
            .unwrap_err()
            .to_string()
            .contains("platformDifferences")
    );
    edit(&bundle, |value| {
        value["platformDifferences"] = json!("declared")
    });
    repository.publish(&bundle).unwrap();
}

#[test]
fn invalid_claim_sets_are_refused_even_on_foreign_platforms() {
    for (member, invalid) in [
        ("claimsVersion", json!("9.0.0")),
        ("critical", json!(["future.required"])),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut declared = claims();
        declared[member] = invalid;
        let bundle = bundle(temp.path(), &[declared]);
        let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
        assert!(
            repository
                .publish(&bundle)
                .unwrap_err()
                .to_string()
                .contains(member)
        );
    }
    let temp = tempfile::tempdir().unwrap();
    let mut declared = claims();
    declared["extension"]["types"] = json!(["future"]);
    let bundle = bundle(temp.path(), &[declared]);
    let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
    assert!(
        repository
            .publish(&bundle)
            .unwrap_err()
            .to_string()
            .contains("future")
    );
}

#[test]
#[cfg(unix)]
fn foreign_process_checksum_and_duplicate_artifacts_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = bundle(temp.path(), &[claims()]);
    let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
    fs::write(bundle.join("guest-0.sha256"), b"wrong checksum").unwrap();
    assert!(
        repository
            .publish(&bundle)
            .unwrap_err()
            .to_string()
            .contains("checksum")
    );
    let digest = Sha256Digest::of_bytes(b"guest");
    fs::write(
        bundle.join("guest-0.sha256"),
        format!("{digest}  guest-0\n"),
    )
    .unwrap();
    edit(&bundle, |value| {
        let duplicate = value["artifacts"][0].clone();
        value["artifacts"].as_array_mut().unwrap().push(duplicate);
    });
    assert!(
        repository
            .publish(&bundle)
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
}

#[test]
#[cfg(unix)]
fn optional_claims_differences_must_be_declared() {
    let temp = tempfile::tempdir().unwrap();
    let mut other = claims();
    other["future"] = json!({"enabled": true});
    let bundle = bundle(temp.path(), &[claims(), other]);
    let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
    let error = repository.publish(&bundle).unwrap_err().to_string();
    assert!(
        error.contains("platformDifferences") && error.contains("future"),
        "{error}"
    );
}

#[test]
#[cfg(unix)]
fn every_digest_is_checked_before_a_host_probe_and_verified_bytes_are_published() {
    use morphir_distribution::{DistributionError, PublicationDescription};
    let temp = tempfile::tempdir().unwrap();
    let bundle = bundle(temp.path(), &[claims(), claims()]);
    let suffix = match std::env::consts::OS {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-gnu",
        other => panic!("unsupported test host {other}"),
    };
    edit(&bundle, |value| {
        value["artifacts"][0]["platform"] = json!(format!("{}-{suffix}", std::env::consts::ARCH))
    });
    let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
    fs::write(bundle.join("guest-1"), b"tampered").unwrap();
    let error = repository
        .publish_with_process_probe(&bundle, |_, _| {
            panic!("must verify all digests before probing")
        })
        .unwrap_err();
    assert!(matches!(error, DistributionError::DigestMismatch { .. }));
    assert_eq!(
        fs::read_dir(repository.root().join("artifacts"))
            .unwrap()
            .count(),
        0
    );
    fs::write(bundle.join("guest-1"), b"guest").unwrap();
    assert!(
        repository
            .publish(&bundle)
            .unwrap_err()
            .to_string()
            .contains("requires a describe probe")
    );
    let publication = repository
        .publish_with_process_probe(&bundle, |artifact, bytes| {
            assert_eq!(bytes, b"guest");
            fs::write(
                bundle.join(artifact.filename().as_str()),
                b"changed after verification",
            )
            .unwrap();
            Ok(PublicationDescription::Describe(artifact.claims().clone()))
        })
        .unwrap();
    assert_eq!(fs::read(publication.artifact_path()).unwrap(), b"guest");
}

#[test]
#[cfg(unix)]
fn republish_from_another_host_ignores_provenance() {
    use morphir_distribution::{PublicationDescription, PublicationStatus};
    let temp = tempfile::tempdir().unwrap();
    let bundle = bundle(temp.path(), &[claims(), claims()]);
    let suffix = if std::env::consts::OS == "macos" {
        "apple-darwin"
    } else {
        "unknown-linux-gnu"
    };
    edit(&bundle, |value| {
        value["artifacts"][1]["platform"] = json!(format!("{}-{suffix}", std::env::consts::ARCH))
    });
    let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
    let describe = |artifact: &morphir_distribution::BundleArtifactDescriptor, _: &[u8]| {
        Ok(PublicationDescription::Describe(artifact.claims().clone()))
    };
    repository
        .publish_with_process_probe(&bundle, describe)
        .unwrap();
    let history = repository.root().join("extensions/example.jsonl");
    let mut host_a: Value = serde_json::from_slice(&fs::read(&history).unwrap()).unwrap();
    // The same release as recorded by the other host, which probed guest-0.
    host_a["artifacts"][0]["claimCheck"] = json!("probed");
    host_a["artifacts"][0]["probeSource"] = json!("session-fallback");
    host_a["artifacts"][1]["claimCheck"] = json!("unchecked");
    host_a["artifacts"][1]
        .as_object_mut()
        .unwrap()
        .remove("probeSource");
    fs::write(&history, serde_json::to_vec(&host_a).unwrap()).unwrap();
    let publication = repository
        .publish_with_process_probe(&bundle, describe)
        .unwrap();
    assert_eq!(publication.status(), PublicationStatus::AlreadyPresent);
    let stored: Value = serde_json::from_slice(&fs::read(&history).unwrap()).unwrap();
    assert_eq!(stored["artifacts"][0]["claimCheck"], "probed");
    assert_eq!(stored["artifacts"][0]["probeSource"], "session-fallback");
    assert_eq!(stored["artifacts"][1]["claimCheck"], "probed");
    assert_eq!(stored["artifacts"][1]["probeSource"], "describe");
    assert_eq!(serde_json::to_value(publication.release()).unwrap(), stored);
    let upgraded = fs::read(&history).unwrap();
    let repeated = repository
        .publish_with_process_probe(&bundle, describe)
        .unwrap();
    assert_eq!(serde_json::to_value(repeated.release()).unwrap(), stored);
    assert_eq!(fs::read(&history).unwrap(), upgraded);
    edit(&bundle, |value| {
        for artifact in value["artifacts"].as_array_mut().unwrap() {
            artifact["claims"]["future"] = json!(true);
        }
    });
    assert!(
        repository
            .publish_with_process_probe(&bundle, describe)
            .is_err()
    );
}

#[test]
#[cfg(unix)]
fn architecture_aliases_use_rust_names() {
    for (input, expected) in [
        ("arm64", "aarch64"),
        ("i686", "x86"),
        ("i586", "x86"),
        ("i386", "x86"),
        ("amd64", "x86_64"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let bundle = bundle(temp.path(), &[claims()]);
        edit(&bundle, |value| {
            value["artifacts"][0]["platform"] = json!(format!("{input}-pc-windows-msvc"))
        });
        let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
        let published = repository.publish(&bundle).unwrap();
        assert_eq!(
            serde_json::to_value(published.release()).unwrap()["artifacts"][0]["platform"]["arch"],
            expected
        );
    }
}

#[test]
fn unknown_architecture_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = bundle(temp.path(), &[claims()]);
    edit(&bundle, |value| {
        value["artifacts"][0]["platform"] = json!("mystery-pc-windows-msvc")
    });
    let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
    let error = repository.publish(&bundle).unwrap_err().to_string();
    assert!(
        error.contains("unsupported process architecture 'mystery'"),
        "{error}"
    );
}

#[test]
fn archives_are_refused_before_any_probe() {
    for suffix in ["tgz", "tar.gz", "zip", "tar"] {
        let temp = tempfile::tempdir().unwrap();
        let bundle = bundle(temp.path(), &[claims(), claims()]);
        let filename = format!("guest-1.{suffix}");
        fs::rename(bundle.join("guest-1"), bundle.join(&filename)).unwrap();
        fs::remove_file(bundle.join("guest-1.sha256")).unwrap();
        fs::write(
            bundle.join(format!("{filename}.sha256")),
            format!("{}  {filename}\n", Sha256Digest::of_bytes(b"guest")),
        )
        .unwrap();
        edit(&bundle, |value| {
            let os = if std::env::consts::OS == "macos" {
                "apple-darwin"
            } else {
                "unknown-linux-gnu"
            };
            value["artifacts"][0]["platform"] = json!(format!("{}-{os}", std::env::consts::ARCH));
            value["artifacts"][1]["filename"] = json!(filename);
        });
        let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
        let error = repository
            .publish_with_process_probe(&bundle, |_, _| {
                panic!("archive refusal must precede probes")
            })
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(&format!(
                "publish supports raw executables only; `{filename}` is an archive"
            )),
            "{error}"
        );
    }
}

#[test]
#[cfg(unix)]
fn destination_conflict_leaves_no_partial_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = bundle(temp.path(), &[claims(), claims()]);
    let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
    fs::write(
        repository.root().join("artifacts/guest-1"),
        b"existing different bytes",
    )
    .unwrap();
    assert!(repository.publish(&bundle).is_err());
    assert!(!repository.root().join("artifacts/guest-0").exists());
    assert!(!repository.root().join("extensions/example.jsonl").exists());
    assert_eq!(
        fs::read(repository.root().join("artifacts/guest-1")).unwrap(),
        b"existing different bytes"
    );
}

#[test]
fn version_two_wasm_keeps_declared_claims_without_probe() {
    let temp = tempfile::tempdir().unwrap();
    let mut declared = claims();
    declared["future"] = json!({"retained": true});
    let bundle = bundle(temp.path(), &[declared.clone()]);
    edit(&bundle, |value| {
        value["artifacts"][0]["runtime"] = json!("wasm");
        value["artifacts"][0]
            .as_object_mut()
            .unwrap()
            .remove("platform");
    });
    let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
    let published = repository
        .publish_with_process_probe(&bundle, |_, _| panic!("WASM must not probe"))
        .unwrap();
    let wire = serde_json::to_value(published.release()).unwrap();
    assert_eq!(wire["artifacts"][0]["claims"], declared);
    assert_eq!(wire["artifacts"][0]["claimCheck"], "unchecked");
    assert!(wire["artifacts"][0].get("probeSource").is_none());
}

#[test]
fn mixed_runtimes_are_refused_before_probing() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = bundle(temp.path(), &[claims(), claims()]);
    edit(&bundle, |value| {
        value["artifacts"][0]["runtime"] = json!("wasm");
        value["artifacts"][0]
            .as_object_mut()
            .unwrap()
            .remove("platform");
    });
    let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
    let error = repository
        .publish_with_process_probe(&bundle, |_, _| panic!("must refuse before probing"))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("a bundle holds either WASM artifacts or process artifacts, not both"),
        "{error}"
    );
    assert!(!repository.root().join("extensions/example.jsonl").exists());
}

#[test]
fn unrepresentable_target_triples_are_refused() {
    for triple in [
        "x86_64-unknown-linux-musl",
        "x86_64-pc-windows-gnu",
        "powerpc64-unknown-linux-gnu",
        "aarch64_be-unknown-linux-gnu",
        "x86_64-unknown-linux-gnux32",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let bundle = bundle(temp.path(), &[claims()]);
        edit(&bundle, |value| {
            value["artifacts"][0]["platform"] = json!(triple)
        });
        let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
        let error = repository
            .publish_with_process_probe(&bundle, |_, _| panic!("must refuse before probing"))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(triple) && error.contains("this index cannot represent it yet"),
            "{error}"
        );
    }
}

#[test]
fn linux_gnu_target_is_accepted() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = bundle(temp.path(), &[claims()]);
    edit(&bundle, |value| {
        value["artifacts"][0]["platform"] = json!("x86_64-unknown-linux-gnu")
    });
    let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
    let publication = repository
        .publish_with_process_probe(&bundle, |artifact, _| {
            Ok(morphir_distribution::PublicationDescription::Describe(
                artifact.claims().clone(),
            ))
        })
        .unwrap();
    assert_eq!(
        serde_json::to_value(publication.release()).unwrap()["artifacts"][0]["platform"],
        json!({"os":"linux", "arch":"x86_64"})
    );
}

#[test]
#[cfg(unix)]
fn already_present_returns_stored_provenance_without_rewriting_history() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = bundle(temp.path(), &[claims()]);
    let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
    repository.publish(&bundle).unwrap();
    let history = repository.root().join("extensions/example.jsonl");
    let mut stored: Value = serde_json::from_slice(&fs::read(&history).unwrap()).unwrap();
    stored["artifacts"][0]["claimCheck"] = json!("probed");
    stored["artifacts"][0]["probeSource"] = json!("describe");
    let bytes = serde_json::to_vec_pretty(&stored).unwrap();
    // Keep JSONL valid while changing whitespace so a rewrite is observable.
    let bytes: Vec<_> = bytes.into_iter().filter(|byte| *byte != b'\n').collect();
    fs::write(&history, &bytes).unwrap();
    let publication = repository.publish(&bundle).unwrap();
    assert_eq!(
        publication.status(),
        morphir_distribution::PublicationStatus::AlreadyPresent
    );
    assert_eq!(serde_json::to_value(publication.release()).unwrap(), stored);
    assert_eq!(fs::read(history).unwrap(), bytes);
}

#[test]
#[cfg(unix)]
fn descriptor_critical_paths_survive_publication() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = bundle(temp.path(), &[claims()]);
    edit(&bundle, |value| {
        value["requires"] = json!({"host": [">=0.1.0"]});
        value["critical"] = json!([
            "extensionId",
            "artifacts.claims.capabilities.backend.generate",
            "requires.host"
        ]);
    });
    let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
    let publication = repository.publish(&bundle).unwrap();
    assert_eq!(
        serde_json::to_value(publication.release()).unwrap()["critical"],
        json!([
            "id",
            "artifacts.claims.capabilities.backend.generate",
            "requires.host"
        ])
    );
}

#[test]
fn descriptor_critical_paths_without_record_members_are_refused() {
    for path in [
        "shortId",
        "gitCommit",
        "platformDifferences",
        "artifacts.requires.host",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let bundle = bundle(temp.path(), &[claims()]);
        edit(&bundle, |value| value["critical"] = json!([path]));
        let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
        let error = repository
            .publish_with_process_probe(&bundle, |_, _| panic!("must refuse before probing"))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(path)
                && error.contains("no corresponding member in the published record"),
            "{error}"
        );
        assert!(!repository.root().join("extensions/example.jsonl").exists());
    }
}

#[test]
fn canonical_little_endian_gnu_architectures_remain_supported() {
    for (arch, expected) in [
        ("powerpc64le", "powerpc64"),
        ("riscv64gc", "riscv64"),
        ("loongarch64", "loongarch64"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let bundle = bundle(temp.path(), &[claims()]);
        edit(&bundle, |value| {
            value["artifacts"][0]["platform"] = json!(format!("{arch}-unknown-linux-gnu"))
        });
        let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
        let publication = repository
            .publish_with_process_probe(&bundle, |artifact, _| {
                Ok(morphir_distribution::PublicationDescription::Describe(
                    artifact.claims().clone(),
                ))
            })
            .unwrap();
        assert_eq!(
            serde_json::to_value(publication.release()).unwrap()["artifacts"][0]["platform"],
            json!({"os":"linux", "arch":expected})
        );
    }
}

#[cfg(unix)]
#[path = "process_publication/compatibility.rs"]
mod compatibility;

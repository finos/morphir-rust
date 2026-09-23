use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const SELECTOR: &[&str] = &["package-mvp"];
const PROFILE: &str = "local-library-mvp:0.1.0-draft.1";

fn exchange(input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mck-adapter-rust"))
        .args(SELECTOR)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mvp-fresh-restore")
}

fn files() -> Vec<Value> {
    fn collect(root: &Path, directory: &Path, found: &mut Vec<Value>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                collect(root, &path, found);
            } else {
                if path.file_name().unwrap() == "bad-timestamp-signature.json" {
                    continue;
                }
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('\\', "/");
                let bytes = fs::read(&path).unwrap();
                let hex = bytes
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                found.push(json!({"path":name,"hex":hex}));
            }
        }
    }
    let root = fixture();
    let mut found = Vec::new();
    collect(&root, &root, &mut found);
    found.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    assert_eq!(found.len(), 15);
    found
}

fn restore(files: Vec<Value>) -> Output {
    let request = json!({"id":2,"op":"restore-local-library","profile":PROFILE,"files":files});
    exchange(&format!(
        "{{\"id\":1,\"op\":\"capabilities\"}}\n{request}\n"
    ))
}

fn refresh(files: Vec<Value>) -> Output {
    let request = json!({"id":2,"op":"refresh-local-library","profile":PROFILE,"files":files});
    exchange(&format!("{request}\n"))
}

fn update_files() -> Vec<Value> {
    fn collect(root: &Path, directory: &Path, found: &mut Vec<Value>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                collect(root, &path, found);
            } else {
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('\\', "/");
                let hex = fs::read(path)
                    .unwrap()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                found.push(json!({"path":name,"hex":hex}));
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../morphir-package/tests/local_registry/update-fixture");
    let mut found = Vec::new();
    for input in ["morphir.lock", "trust-policy.json"] {
        let bytes = fs::read(root.join(input)).unwrap();
        let hex = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        found.push(json!({"path":input,"hex":hex}));
    }
    collect(&root, &root.join("registry"), &mut found);
    found.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    assert_eq!(found.len(), 50);
    found
}

fn update_files_with_metadata_variant(variant: &str) -> Vec<Value> {
    let mut found = update_files();
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../morphir-package/tests/local_registry/update-fixture/variants")
        .join(variant);
    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let name = format!("registry/metadata/{}", entry.file_name().to_str().unwrap());
        let hex = fs::read(entry.path())
            .unwrap()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        found.iter_mut().find(|file| file["path"] == name).unwrap()["hex"] = json!(hex);
    }
    found
}

fn update_files_with_json_edit(path: &str, edit: impl FnOnce(&mut Value)) -> Vec<Value> {
    let mut found = update_files();
    let file = found.iter_mut().find(|file| file["path"] == path).unwrap();
    let bytes = file["hex"].as_str().unwrap();
    let bytes = (0..bytes.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&bytes[index..index + 2], 16).unwrap())
        .collect::<Vec<_>>();
    let mut document: Value = serde_json::from_slice(&bytes).unwrap();
    edit(&mut document);
    file["hex"] = json!(
        serde_json::to_vec_pretty(&document)
            .unwrap()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    found
}

fn update_files_with_content_edit(path: &str, edit: impl FnOnce(&mut Vec<u8>)) -> Vec<Value> {
    let mut found = update_files();
    let file = found.iter_mut().find(|file| file["path"] == path).unwrap();
    let hex = file["hex"].as_str().unwrap();
    let mut bytes = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
        .collect::<Vec<_>>();
    edit(&mut bytes);
    file["hex"] = json!(
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    found
}

fn assert_update_refusal(files: Vec<Value>, targets: &[&str], category: &str, reason: &str) {
    assert_update_refusal_in(
        files,
        targets,
        json!({"trustState":"initialized","output":"absent"}),
        category,
        reason,
    );
}

fn assert_update_refusal_in(
    files: Vec<Value>,
    targets: &[&str],
    environment: Value,
    category: &str,
    reason: &str,
) {
    let request = json!({
        "id":2,"op":"update-local-library","profile":PROFILE,
        "targets":targets,"files":files,"environment":environment
    });
    let answer = responses(&exchange(&format!("{request}\n")));
    let sentinel = environment["output"] == "sentinel";
    assert_eq!(
        answer,
        vec![json!({
            "id":2,"outcome":"refused","category":category,"reason":reason,
            "output":if sentinel { "preserved-sentinel" } else { "absent" },
            "outputFiles":if sentinel { vec![json!({"path":"morphir.lock","sha256":"sha256:f01f017ba20623e8154cdcd63fb16d79795bc0d6f57271934e5a9864b772d425"})] } else { vec![] },
            "lockUnchanged":true,"registryUnchanged":true
        })]
    );
}

fn assert_update_success(files: Vec<Value>, targets: &[&str], digest: &str) {
    let request = json!({
        "id":2,"op":"update-local-library","profile":PROFILE,
        "targets":targets,"files":files
    });
    let answer = responses(&exchange(&format!("{request}\n")));
    assert_eq!(
        answer,
        vec![json!({
            "id":2,"outcome":"updated","output":"present",
            "outputFiles":[{"path":"morphir.lock","sha256":format!("sha256:{digest}")}],
            "lockUnchanged":true,"registryUnchanged":true
        })]
    );
}

#[test]
fn updates_the_independently_frozen_scoped_lock() {
    let request = json!({
        "id":2,"op":"update-local-library","profile":PROFILE,
        "targets":["example.com/finance/eligibility"],"files":update_files()
    });
    let answer = responses(&exchange(&format!("{request}\n")));
    assert_eq!(
        answer,
        vec![json!({
            "id":2,"outcome":"updated","output":"present",
            "outputFiles":[{"path":"morphir.lock","sha256":"sha256:7190428c25cd4b927c5db8bb9fbcbe936c81cfacc679734309a76424d332321a"}],
            "lockUnchanged":true,"registryUnchanged":true
        })]
    );
}

#[test]
fn update_refuses_empty_targets_without_publishing_a_lock() {
    let request = json!({
        "id":2,"op":"update-local-library","profile":PROFILE,
        "targets":[],"files":update_files()
    });
    let answer = responses(&exchange(&format!("{request}\n")));
    assert_eq!(
        answer,
        vec![json!({
            "id":2,"outcome":"refused","category":"invalid-input","reason":"invalid-update-targets",
            "output":"absent","outputFiles":[],"lockUnchanged":true,"registryUnchanged":true
        })]
    );
}

#[test]
fn update_refuses_a_change_outside_the_old_dependency_closure() {
    let request = json!({
        "id":2,"op":"update-local-library","profile":PROFILE,
        "targets":["example.com/finance/eligibility@1.4.0"],
        "files":update_files_with_metadata_variant("scope-conflict")
    });
    let answer = responses(&exchange(&format!("{request}\n")));
    assert_eq!(
        answer,
        vec![json!({
            "id":2,"outcome":"refused","category":"resolution","reason":"update-scope-conflict",
            "output":"absent","outputFiles":[],"lockUnchanged":true,"registryUnchanged":true
        })]
    );
}

#[test]
fn update_refuses_malformed_and_ineligible_targets() {
    let path = "example.com/finance/eligibility";
    for targets in [
        vec!["../invalid"],
        vec!["example.com/finance/loan-rules"],
        vec!["example.com/finance/missing"],
        vec![path, path],
    ] {
        assert_update_refusal(
            update_files(),
            &targets,
            "invalid-input",
            "invalid-update-targets",
        );
    }
    for targets in [
        vec!["example.com/finance/eligibility@9.9.9"],
        vec!["example.com/finance/eligibility@1.9.0"],
    ] {
        assert_update_refusal(
            update_files(),
            &targets,
            "resolution",
            "unsatisfiable-requirements",
        );
    }
}

#[test]
fn update_refuses_revocation_and_invalid_old_graph() {
    assert_update_refusal(
        update_files_with_metadata_variant("revoked-frozen"),
        &["example.com/finance/eligibility"],
        "unsupported-policy",
        "revocation-transition-unsupported",
    );
    let files = update_files_with_json_edit("morphir.lock", |lock| {
        lock["graph"]["nodes"][0]["bindings"][0]["target"]["version"] = "9.9.9".into();
    });
    assert_update_refusal(
        files,
        &["example.com/finance/eligibility"],
        "invalid-input",
        "invalid-old-lock",
    );
}

#[test]
fn update_preserves_frozen_scoped_outcomes_across_target_forms_and_statuses() {
    let target = "example.com/finance/eligibility";
    let common = "7190428c25cd4b927c5db8bb9fbcbe936c81cfacc679734309a76424d332321a";
    for targets in [
        vec![target, "example.com/finance/child@1.1.0"],
        vec!["example.com/finance/child@1.1.0", target],
        vec!["example.com/finance/eligibility@1.3.0"],
    ] {
        assert_update_success(update_files(), &targets, common);
    }
    assert_update_success(
        update_files(),
        &["example.com/finance/eligibility@1.2.0"],
        "062762de6ffa30a63a03cb8d8a7b8fd63ed1f20c4a69789ade3310c3aa64ca6f",
    );
    assert_update_success(
        update_files_with_metadata_variant("yanked-frozen"),
        &[target],
        "3380a67664c1987595b9a6a97cfa4c40094193e4fefe33e3fa0f1dbf6ff91fe8",
    );
    assert_update_success(
        update_files_with_metadata_variant("yanked-root"),
        &[target],
        "10ba4fd1d245432fe94d9fea08edf65449988d1b0edfc13f9abdb044c2c09810",
    );
}

#[test]
fn update_refuses_old_pin_mismatches() {
    let target = &["example.com/finance/eligibility"][..];
    let acquisition = update_files_with_json_edit("morphir.lock", |lock| {
        lock["acquisitions"][0]["record"]["digest"] = format!("sha256:{}", "00".repeat(32)).into();
    });
    assert_update_refusal(
        acquisition,
        target,
        "invalid-input",
        "old-record-acquisition-mismatch",
    );
    let statement = update_files_with_json_edit("morphir.lock", |lock| {
        let evidence = lock["evidence"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|e| e["kind"] == "release-statement")
            .unwrap();
        evidence["digest"] = format!("sha256:{}", "00".repeat(32)).into();
    });
    assert_update_refusal(
        statement,
        target,
        "invalid-input",
        "old-record-acquisition-mismatch",
    );
}

#[test]
fn update_refuses_authentication_and_state_failures() {
    let target = &["example.com/finance/eligibility"][..];
    let invalid_signature =
        update_files_with_json_edit("registry/metadata/2.timestamp.json", |timestamp| {
            timestamp["signatures"][0]["sig"] = "00".repeat(64).into();
        });
    let mut invalid_signature = invalid_signature;
    let changed = invalid_signature
        .iter()
        .find(|file| file["path"] == "registry/metadata/2.timestamp.json")
        .unwrap()["hex"]
        .clone();
    invalid_signature
        .iter_mut()
        .find(|file| file["path"] == "registry/metadata/timestamp.json")
        .unwrap()["hex"] = changed;
    assert_update_refusal(
        invalid_signature,
        target,
        "metadata-authentication",
        "timestamp-signature-threshold",
    );
    for (trust_state, reason) in [
        ("uninitialized", "uninitialized"),
        ("missing-database", "missing-established-database"),
        ("corrupt-database", "corrupt-established-database"),
        ("unresolved-operation", "unresolved-operation"),
    ] {
        assert_update_refusal_in(
            update_files(),
            target,
            json!({"trustState":trust_state,"output":"absent"}),
            "trust-state",
            reason,
        );
    }
    assert_update_refusal_in(
        update_files(),
        target,
        json!({"trustState":"initialized","output":"sentinel"}),
        "output-conflict",
        "destination-exists",
    );
}

#[test]
fn update_refuses_expired_metadata_and_tampered_frozen_content() {
    let target = &["example.com/finance/eligibility"][..];
    let expired = fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../morphir-package/tests/local_registry/update-fixture/variants/expired-timestamp.json",
    ))
    .unwrap();
    let expired_hex = expired
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut files = update_files();
    for name in [
        "registry/metadata/2.timestamp.json",
        "registry/metadata/timestamp.json",
    ] {
        files.iter_mut().find(|file| file["path"] == name).unwrap()["hex"] = json!(expired_hex);
    }
    assert_update_refusal(
        files,
        target,
        "metadata-authentication",
        "timestamp-expired",
    );

    let lock: Value = serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../morphir-package/tests/local_registry/update-fixture/morphir.lock"),
        )
        .unwrap(),
    )
    .unwrap();
    let acquisition = lock["acquisitions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["release"]["packagePath"] == "example.com/finance/sibling")
        .unwrap();
    let path = format!(
        "registry/{}/ir.json",
        acquisition["source"]["path"].as_str().unwrap()
    );
    let files = update_files_with_content_edit(&path, |bytes| bytes.push(b'\n'));
    assert_update_refusal(
        files,
        target,
        "package-integrity",
        "content-digest-mismatch",
    );
}

#[test]
fn refresh_rejects_an_unmodeled_consumer_output_setup() {
    let request = json!({
        "id":2,"op":"refresh-local-library","profile":PROFILE,"files":files(),
        "environment":{"trustState":"initialized","output":"sentinel"}
    });
    let output = exchange(&format!("{request}\n"));
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("metadata-only refresh requires absent consumer output")
    );
}

fn responses(output: &Output) -> Vec<Value> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}

#[test]
fn advertises_the_fresh_restore_resolve_refresh_and_update_operations() {
    let answer = responses(&exchange("{\"id\":1,\"op\":\"capabilities\"}\n"));
    assert_eq!(
        answer,
        vec![json!({
            "id":1,"suite":"package","contractVersion":"0.1.0-draft.3",
            "implementation":"morphir-rust","implementationVersion":env!("CARGO_PKG_VERSION"),
            "profiles":[PROFILE],"operations":["restore-local-library","resolve-local-library","refresh-local-library","update-local-library"]
        })]
    );
}

#[test]
fn resolves_the_signed_graph_to_the_independently_frozen_lock_digest() {
    let request = json!({
        "id":2,"op":"resolve-local-library","profile":PROFILE,"files":files(),
        "exactRoot":"example.com/finance/loan-rules@1.0.0"
    });
    let answer = responses(&exchange(&format!("{request}\n")));
    assert_eq!(
        answer,
        vec![json!({
            "id":2,"outcome":"resolved","output":"present",
            "outputFiles":[{"path":"morphir.lock","sha256":"sha256:2db3c346d885528c3ef46d60ed9c563d5f6ba45992ef2f6a3c7dfc294d5b8f24"}],
            "lockUnchanged":true,"registryUnchanged":true
        })]
    );
}

#[test]
fn refresh_authenticates_only_the_frozen_metadata_envelopes() {
    let answer = responses(&refresh(files()));
    assert_eq!(
        answer,
        vec![json!({
            "id":2,"outcome":"refreshed","output":"absent","outputFiles":[],
            "receipt":{
                "profile":"local-library-mvp","profileVersion":"0.1.0-draft.1",
                "registry":"local",
                "timestampDigest":"sha256:42ef09963e40de1cff6de42d5680d4c67e9f337693fb804af2873f5ee4d19636",
                "snapshotDigest":"sha256:9cc590f7328e43e14597bab4bfc82e2192457e7bec07684d63a622b8f97bbe0e"
            },
            "lockUnchanged":true,"registryUnchanged":true
        })]
    );
}

#[test]
fn refresh_does_not_require_package_bundles_records_or_publisher_envelopes() {
    let expected = responses(&refresh(files()))[0].clone();
    for prefix in [
        "registry/bundles/",
        "registry/targets/records/",
        "registry/targets/statements/",
    ] {
        let mut input = files();
        input.retain(|file| !file["path"].as_str().unwrap().starts_with(prefix));
        assert_eq!(responses(&refresh(input))[0], expected, "{prefix}");
    }
}

#[test]
fn refresh_rejects_invalid_timestamp_signature_without_a_receipt() {
    let mut input = files();
    let timestamp = input
        .iter_mut()
        .find(|file| file["path"] == "registry/metadata/timestamp.json")
        .unwrap();
    let invalid = fs::read(fixture().join("bad-timestamp-signature.json")).unwrap();
    timestamp["hex"] = Value::String(invalid.iter().map(|byte| format!("{byte:02x}")).collect());
    assert_eq!(
        responses(&refresh(input))[0],
        json!({
            "id":2,"outcome":"refused","category":"metadata-authentication",
            "reason":"timestamp-signature-threshold","output":"absent","outputFiles":[],
            "lockUnchanged":true,"registryUnchanged":true
        })
    );
}

#[test]
fn unavailable_exact_root_is_a_typed_refusal_without_a_partial_lock() {
    let request = json!({
        "id":2,"op":"resolve-local-library","profile":PROFILE,"files":files(),
        "exactRoot":"example.com/finance/loan-rules@9.9.9"
    });
    let answer = responses(&exchange(&format!("{request}\n")));
    assert_eq!(
        answer,
        vec![json!({
            "id":2,"outcome":"refused","category":"invalid-input","reason":"published-root-unavailable",
            "output":"absent","outputFiles":[],"lockUnchanged":true,"registryUnchanged":true
        })]
    );
}

#[test]
fn restores_signed_graph_through_production_api() {
    let answer = responses(&restore(files()));
    assert_eq!(answer.len(), 2);
    assert_eq!(
        answer[1],
        json!({
            "id":2,"outcome":"restored","output":"present",
            "lockUnchanged":true,"registryUnchanged":true,
            "packages":[
                {"packagePath":"example.com/finance/eligibility","version":"1.2.0","directory":"example.com/finance/eligibility/1.2.0"},
                {"packagePath":"example.com/finance/loan-rules","version":"1.0.0","directory":"example.com/finance/loan-rules/1.0.0"}
            ],
            "outputFiles":[
                {"path":"example.com/finance/eligibility/1.2.0/ir.json","sha256":"sha256:243e640848e5ee728224c0bc9090c67cf434d9814cec77745daad918119ceb43"},
                {"path":"example.com/finance/eligibility/1.2.0/manifest.json","sha256":"sha256:0a91b5e3a5e377fa4557212fc5b561a66067d67292123136575e46ce5a3fd0b7"},
                {"path":"example.com/finance/loan-rules/1.0.0/ir.json","sha256":"sha256:b1884eeb8364f96c6405cc45f2fe07a006c22f066a4136656ad05dda9c9a94cc"},
                {"path":"example.com/finance/loan-rules/1.0.0/manifest.json","sha256":"sha256:e98ea924e66b96a5050e2c3a690f9534db32f5971401d2965791546872ecb7a3"}
            ]
        })
    );
}

#[test]
fn invalid_timestamp_signature_is_the_narrow_refusal() {
    let mut files = files();
    let timestamp = files
        .iter_mut()
        .find(|file| file["path"] == "registry/metadata/timestamp.json")
        .unwrap();
    let invalid = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mvp-fresh-restore/bad-timestamp-signature.json"),
    )
    .unwrap();
    timestamp["hex"] = Value::String(invalid.iter().map(|byte| format!("{byte:02x}")).collect());
    let answer = responses(&restore(files));
    assert_eq!(
        answer[1],
        json!({
            "id":2,"outcome":"refused","category":"metadata-authentication",
            "reason":"timestamp-signature-threshold",
            "output":"absent","outputFiles":[],"lockUnchanged":true,"registryUnchanged":true
        })
    );
}

#[test]
fn uninitialized_state_is_a_typed_refusal_with_no_output() {
    let request = json!({
        "id":2,"op":"restore-local-library","profile":PROFILE,"files":files(),
        "environment":{"trustState":"uninitialized","output":"absent"}
    });
    let answer = responses(&exchange(&format!(
        "{{\"id\":1,\"op\":\"capabilities\"}}\n{request}\n"
    )));
    assert_eq!(
        answer[1],
        json!({
            "id":2,"outcome":"refused","category":"trust-state","reason":"uninitialized",
            "output":"absent","outputFiles":[],"lockUnchanged":true,"registryUnchanged":true
        })
    );
}

#[test]
fn established_state_and_occupied_output_refusals_preserve_inputs() {
    for (trust_state, output, category, reason) in [
        (
            "missing-database",
            "absent",
            "trust-state",
            "missing-established-database",
        ),
        (
            "corrupt-database",
            "absent",
            "trust-state",
            "corrupt-established-database",
        ),
        (
            "unresolved-operation",
            "absent",
            "trust-state",
            "unresolved-operation",
        ),
        (
            "initialized",
            "sentinel",
            "output-conflict",
            "destination-exists",
        ),
    ] {
        let request = json!({
            "id":2,"op":"restore-local-library","profile":PROFILE,"files":files(),
            "environment":{"trustState":trust_state,"output":output}
        });
        let answer = responses(&exchange(&format!("{request}\n")));
        let expected_files = if output == "sentinel" {
            json!([{"path":"sentinel.txt","sha256":"sha256:f01f017ba20623e8154cdcd63fb16d79795bc0d6f57271934e5a9864b772d425"}])
        } else {
            json!([])
        };
        assert_eq!(
            answer[0],
            json!({
                "id":2,"outcome":"refused","category":category,"reason":reason,
                "output":if output == "sentinel" { "preserved-sentinel" } else { "absent" },
                "outputFiles":expected_files,"lockUnchanged":true,"registryUnchanged":true
            }),
            "{trust_state}/{output}"
        );
    }
}

#[test]
fn bounded_file_variants_produce_independent_refusals() {
    let mut bad_content = files();
    let ir = bad_content
        .iter_mut()
        .find(|file| file["path"].as_str().unwrap().ends_with("/ir.json"))
        .unwrap();
    ir["hex"] = json!(format!("{}0a", ir["hex"].as_str().unwrap()));

    let mut extra_bundle = files();
    extra_bundle.push(json!({
        "path":"registry/bundles/5922bc8860f6cd008b9cda341be7f3a776ea332e63261392e94c17e19a647886/undeclared.txt",
        "hex":"6e6f7420696e206d616e69666573740a"
    }));

    let mut historical_policy = files();
    let policy = historical_policy
        .iter_mut()
        .find(|file| file["path"] == "trust-policy.json")
        .unwrap();
    let original = policy["hex"].as_str().unwrap().to_owned();
    historical_policy.push(json!({"path":"initialization-policy.json","hex":original}));
    let policy = historical_policy
        .iter_mut()
        .find(|file| file["path"] == "trust-policy.json")
        .unwrap();
    let bytes = (0..original.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&original[index..index + 2], 16).unwrap())
        .collect::<Vec<_>>();
    let mut policy_json: Value = serde_json::from_slice(&bytes).unwrap();
    policy_json["continuedUse"] = json!("previous-authorization");
    policy["hex"] = json!(
        serde_json::to_vec(&policy_json)
            .unwrap()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );

    for (variant, expected_category, expected_reason) in [
        (bad_content, "package-integrity", "content-digest-mismatch"),
        (
            extra_bundle,
            "package-integrity",
            "bundle-inventory-mismatch",
        ),
        (
            historical_policy,
            "unsupported-policy",
            "historical-authorization-unsupported",
        ),
    ] {
        let answer = responses(&restore(variant));
        assert_eq!(
            answer[1],
            json!({
                "id":2,"outcome":"refused","category":expected_category,"reason":expected_reason,
                "output":"absent","outputFiles":[],"lockUnchanged":true,"registryUnchanged":true
            })
        );
    }
}

#[test]
fn malformed_or_incomplete_input_fails_the_protocol() {
    let original = files();
    let mut missing = original.clone();
    missing.pop();
    let mut duplicate = original.clone();
    duplicate.push(original[0].clone());
    let mut traversal = original.clone();
    traversal[0]["path"] = json!("../escape");
    for files in [missing, duplicate, traversal] {
        let request = json!({"id":1,"op":"restore-local-library","profile":PROFILE,"files":files});
        let output = exchange(&format!("{request}\n"));
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    let with_expected_answer = json!({
        "id":1,"op":"restore-local-library","profile":PROFILE,
        "files":original,"expected":{"outcome":"restored"}
    });
    let output = exchange(&format!("{with_expected_answer}\n"));
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn invalid_lock_is_not_reported_as_a_metadata_refusal() {
    let mut files = files();
    let lock = files
        .iter_mut()
        .find(|file| file["path"] == "morphir.lock")
        .unwrap();
    lock["hex"] = json!("7b");
    let request = json!({"id":1,"op":"restore-local-library","profile":PROFILE,"files":files});
    let output = exchange(&format!("{request}\n"));
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("restore failed"));
}

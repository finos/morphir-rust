#![cfg(target_os = "macos")]
use morphir_package::{
    authoring::{AuthoredLibrary, LocalSigningKey},
    local_registry::publication::{Outcome, Predecessor, Registry},
};
use serde_json::{Value, json};

fn setup() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    Vec<u8>,
    LocalSigningKey,
) {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let key = LocalSigningKey::from_seed([21; 32]);
    let id = key.tuf_key_id().unwrap();
    let role = json!({"keyids":[id.clone()],"threshold":1});
    let root=key.sign_tuf(&json!({"_type":"root","spec_version":"1.0.36","version":1,"expires":"2099-01-01T00:00:00Z","consistent_snapshot":true,
        "keys":{id:key.tuf_public_key()},"roles":{"root":role,"targets":role,"snapshot":role,"timestamp":role}})).unwrap();
    let digest = morphir_package::digest::Digest::of_bytes(&root).to_string();
    let policy=serde_json::to_vec(&json!({"formatVersion":"0.1.0-draft.3","kind":"LibraryTrustPolicy",
        "repositories":[{"identity":digest,"bootstrapRoot":{"version":1,"digest":digest},"namespaces":["example.com"]}],
        "publisherRules":[{"namespace":"example.com","publicKeys":[key.public_key_hex()],"threshold":1}],"continuedUse":"fresh-metadata"})).unwrap();
    Registry::initialize(&base.join("registry"), &base.join("state"), &policy, &root).unwrap();
    (temp, base, policy, key)
}
fn library(version: &str) -> AuthoredLibrary {
    let ir = json!({"formatVersion":4,"distribution":{"Library":{"packageName":"example/greeting","dependencies":{},"def":{"modules":{"greeting":{"Public":{"types":{},"values":{}}}}}}}});
    let config = json!({"packagePath":"example.com/greeting","version":version,"dependencies":{},"exports":{"greeting":"greeting"}});
    AuthoredLibrary::create(
        &serde_json::to_vec(&config).unwrap(),
        &serde_json::to_vec(&ir).unwrap(),
    )
    .unwrap()
}

#[test]
fn linked_context_is_published_with_signed_archive_and_tampering_refuses_reopen() {
    let (_temp, base, policy, key) = setup();
    let ir = json!({"formatVersion":"4.1.0","distribution":{"Library":{"packageName":"example/greeting","dependencies":{},"def":{"modules":{"greeting":{"Public":{"types":{},"values":{}}}}}}},
    "$meta":{"@context":"./contexts/names.jsonld","@graph":[{
        "@id":"morphir://ir/pkg/example/greeting?format=4.1.0#/module/greeting",
        "operationalName":"sayHello"
    }]}});
    let input = json!({"packagePath":"example.com/greeting","version":"1.0.0","dependencies":{},"exports":{"greeting":"greeting"}});
    let context = br#"{"@context":{"operationalName":"morphir://ir/pkg/example/greeting?format=4.1.0#/module/greeting/value/operational-name"}}"#;
    let library = AuthoredLibrary::create_with_contexts(
        &serde_json::to_vec(&input).unwrap(),
        &serde_json::to_vec(&ir).unwrap(),
        vec![("contexts/names.jsonld".into(), context.to_vec())],
    )
    .unwrap();
    let signed = library.sign(&key).unwrap();
    let registry = Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    let draft = registry
        .prepare(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            "2098-01-01T00:00:00Z",
        )
        .unwrap();
    registry
        .publish(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            draft.predecessor(),
            &draft.sign(&key, &key, &key).unwrap(),
        )
        .unwrap();
    let digest = library.metadata().content_digest().to_string();
    let stored = base.join(format!(
        "registry/bundles/{}/contexts/names.jsonld",
        &digest[7..]
    ));
    assert_eq!(std::fs::read(&stored).unwrap(), context);
    drop(registry);
    Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    std::fs::write(&stored, b"changed").unwrap();
    assert!(Registry::open(&base.join("registry"), &base.join("state"), &policy).is_err());
}

#[tokio::test]
async fn second_project_restores_authenticated_context_without_authoring_source() {
    use morphir_package::{
        local_registry::mvp::{self, InitializeRequest, ResolveRequest, RestoreRequest},
        resolution::{PackagePath, ReleaseId, StableVersion},
    };
    let (_temp, base, policy, key) = setup();
    let ir = json!({"formatVersion":"4.1.0","distribution":{"Library":{"packageName":"example/greeting","dependencies":{},"def":{"modules":{"greeting":{"Public":{"types":{},"values":{}}}}}}},
    "$meta":{"@context":"./contexts/names.jsonld","@graph":[{
        "@id":"morphir://ir/pkg/example/greeting?format=4.1.0#/module/greeting",
        "operationalName":"sayHello"
    }]}});
    let input = json!({"packagePath":"example.com/greeting","version":"1.0.0","dependencies":{},"exports":{"greeting":"greeting"}});
    let context = br#"{"@context":{"operationalName":"morphir://ir/pkg/example/greeting?format=4.1.0#/module/greeting/value/operational-name"}}"#;
    let library = AuthoredLibrary::create_with_contexts(
        &serde_json::to_vec(&input).unwrap(),
        &serde_json::to_vec(&ir).unwrap(),
        vec![("contexts/names.jsonld".into(), context.to_vec())],
    )
    .unwrap();
    let bindings = morphir_package::authoring::PublicationBindings::new(&library).unwrap();
    assert!(
        bindings
            .bind_uri(
                &morphir_core::node_address::NodeUri::parse(
                    "morphir://ir/pkg/example/greeting?format=4.1.0#/module/greeting"
                )
                .unwrap()
            )
            .is_ok()
    );
    let signed = library.sign(&key).unwrap();
    let publisher = Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    let draft = publisher
        .prepare(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            "2098-01-01T00:00:00Z",
        )
        .unwrap();
    publisher
        .publish(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            draft.predecessor(),
            &draft.sign(&key, &key, &key).unwrap(),
        )
        .unwrap();
    drop(publisher);
    let root = std::fs::read(base.join("registry/metadata/1.root.json")).unwrap();
    let consumer_state = base.join("other-project-trust");
    mvp::initialize(InitializeRequest {
        policy: &policy,
        root: &root,
        state: &consumer_state,
    })
    .unwrap();
    let lock_path = base.join("other-project.lock");
    mvp::resolve(ResolveRequest {
        policy: &policy,
        root: ReleaseId::new(
            PackagePath::parse("example.com/greeting").unwrap(),
            StableVersion::parse("1.0.0").unwrap(),
        ),
        registry: &base.join("registry"),
        state: &consumer_state,
        output: &lock_path,
    })
    .await
    .unwrap();
    let lock = std::fs::read(lock_path).unwrap();
    let output = base.join("other-project-packages");
    let report = mvp::restore(RestoreRequest {
        policy: &policy,
        lock: &lock,
        registry: &base.join("registry"),
        state: &consumer_state,
        output: &output,
    })
    .await
    .unwrap();
    assert_eq!(report.packages.len(), 1);
    assert_eq!(
        std::fs::read(
            output
                .join(&report.packages[0].directory)
                .join("contexts/names.jsonld")
        )
        .unwrap(),
        context
    );
    let restored = output.join(&report.packages[0].directory);
    let ir_bytes = std::fs::read(restored.join("ir.json")).unwrap();
    let mut resources = morphir_core::metadata::ContextResources::new(".");
    resources.insert_local(
        "contexts/names.jsonld",
        std::fs::read(restored.join("contexts/names.jsonld")).unwrap(),
    );
    let authored = morphir_core::ir::json::read(std::str::from_utf8(&ir_bytes).unwrap()).unwrap();
    let inline =
        morphir_core::metadata::inline_document_contexts(&authored, &resources, Some("ir.json"))
            .unwrap();
    let (file, _) = morphir_core::ir::json::read_ir_file(&inline.to_string()).unwrap();
    let graph = morphir_core::ir::v4::expand_v4_single_file_graph(
        &file,
        &morphir_core::metadata::DocumentId::new("ir.json").unwrap(),
        &resources,
        |_| None,
    )
    .unwrap();
    assert_eq!(graph.facts().len(), 1);
    assert_eq!(
        graph.facts()[0].subject().to_string(),
        "morphir://ir/pkg/example/greeting?format=4.1.0#/module/greeting"
    );
    assert_eq!(
        graph.facts()[0].predicate().to_string(),
        "morphir://ir/pkg/example/greeting?format=4.1.0#/module/greeting/value/operational-name"
    );
    let morphir_core::metadata::ObjectTerm::Value(value) = graph.facts()[0].object() else {
        panic!("operational name must remain a value");
    };
    assert_eq!(value.value(), &json!("sayHello"));
}
#[test]
fn first_publication_is_signed_and_retry_requires_exact_predecessor() {
    let (_temp, base, policy, key) = setup();
    let library = library("1.0.0");
    let signed = library.sign(&key).unwrap();
    let registry = Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    let draft = registry
        .prepare(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            "2098-01-01T00:00:00Z",
        )
        .unwrap();
    assert_eq!(draft.predecessor(), &Predecessor::Absent);
    let proposal = draft.sign(&key, &key, &key).unwrap();
    let result = registry
        .publish(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            &Predecessor::Absent,
            &proposal,
        )
        .unwrap();
    assert_eq!(result.outcome, Outcome::Committed);
    assert!(
        registry
            .publish(
                &library,
                signed.record_bytes(),
                signed.envelope_bytes(),
                &Predecessor::Absent,
                &proposal
            )
            .is_err()
    );
    let result = registry
        .publish(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            &Predecessor::Timestamp {
                digest: result.timestamp.clone(),
            },
            &proposal,
        )
        .unwrap();
    assert_eq!(result.outcome, Outcome::Idempotent);
    let timestamp: Value = serde_json::from_slice(
        &std::fs::read(base.join("registry/metadata/timestamp.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(timestamp["signed"]["version"], 1);
}

#[test]
fn invalid_successor_reserves_nothing_and_installs_nothing() {
    use morphir_package::local_registry::publication::Proposal;
    let (_temp, base, policy, key) = setup();
    let library = library("1.0.0");
    let signed = library.sign(&key).unwrap();
    let registry = Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    let proposal = Proposal {
        targets: b"{}".to_vec(),
        snapshot: b"{}".to_vec(),
        timestamp: b"{}".to_vec(),
    };
    assert!(
        registry
            .publish(
                &library,
                signed.record_bytes(),
                signed.envelope_bytes(),
                &Predecessor::Absent,
                &proposal
            )
            .is_err()
    );
    assert_eq!(std::fs::read_dir(base.join("state")).unwrap().count(), 1);
    assert_eq!(
        std::fs::read_dir(base.join("registry/bundles"))
            .unwrap()
            .count(),
        0
    );
    assert!(!base.join("registry/metadata/timestamp.json").exists());
}
#[test]
fn idempotent_retry_ignores_invalid_proposal_and_content_conflict_precedes_proposal() {
    use morphir_package::local_registry::publication::{Error, Proposal};
    let (_temp, base, policy, key) = setup();
    let library = library("1.0.0");
    let signed = library.sign(&key).unwrap();
    let registry = Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    let draft = registry
        .prepare(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            "2098-01-01T00:00:00Z",
        )
        .unwrap();
    let proposal = draft.sign(&key, &key, &key).unwrap();
    let result = registry
        .publish(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            draft.predecessor(),
            &proposal,
        )
        .unwrap();
    let predecessor = Predecessor::Timestamp {
        digest: result.timestamp,
    };
    let invalid = Proposal {
        targets: b"{}".to_vec(),
        snapshot: b"{}".to_vec(),
        timestamp: b"{}".to_vec(),
    };
    assert_eq!(
        registry
            .publish(
                &library,
                signed.record_bytes(),
                signed.envelope_bytes(),
                &predecessor,
                &invalid
            )
            .unwrap()
            .outcome,
        Outcome::Idempotent
    );
    let mut manifest: Value = serde_json::from_slice(library.manifest_bytes()).unwrap();
    manifest["exports"] = json!({});
    let changed =
        AuthoredLibrary::from_bundle(&serde_json::to_vec(&manifest).unwrap(), library.ir_bytes())
            .unwrap();
    let signed = changed.sign(&key).unwrap();
    assert!(matches!(
        registry.publish(
            &changed,
            signed.record_bytes(),
            signed.envelope_bytes(),
            &predecessor,
            &invalid
        ),
        Err(Error::ReleaseConflict)
    ));
}
#[test]
fn missing_committed_timestamp_is_corrupt_state_not_an_empty_registry() {
    let (_temp, base, policy, key) = setup();
    let library = library("1.0.0");
    let signed = library.sign(&key).unwrap();
    {
        let registry =
            Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
        let draft = registry
            .prepare(
                &library,
                signed.record_bytes(),
                signed.envelope_bytes(),
                "2098-01-01T00:00:00Z",
            )
            .unwrap();
        registry
            .publish(
                &library,
                signed.record_bytes(),
                signed.envelope_bytes(),
                draft.predecessor(),
                &draft.sign(&key, &key, &key).unwrap(),
            )
            .unwrap();
    }
    std::fs::remove_file(base.join("registry/metadata/timestamp.json")).unwrap();
    assert!(Registry::open(&base.join("registry"), &base.join("state"), &policy).is_err());
}

#[test]
fn successor_cannot_change_previous_status_or_add_unrequested_targets() {
    let (_temp, base, policy, key) = setup();
    let first = library("1.0.0");
    let signed = first.sign(&key).unwrap();
    let registry = Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    let draft = registry
        .prepare(
            &first,
            signed.record_bytes(),
            signed.envelope_bytes(),
            "2098-01-01T00:00:00Z",
        )
        .unwrap();
    registry
        .publish(
            &first,
            signed.record_bytes(),
            signed.envelope_bytes(),
            draft.predecessor(),
            &draft.sign(&key, &key, &key).unwrap(),
        )
        .unwrap();
    let next = library("1.1.0");
    let signed = next.sign(&key).unwrap();
    let draft = registry
        .prepare(
            &next,
            signed.record_bytes(),
            signed.envelope_bytes(),
            "2098-01-01T00:00:00Z",
        )
        .unwrap();
    let mut changed = serde_json::to_value(&draft).unwrap();
    for pin in changed["targets"]["targets"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        if pin["custom"]["morphir"]["release"]["version"] == "1.0.0" {
            pin["custom"]["morphir"]["status"] = json!("yanked");
        }
    }
    let changed: morphir_package::local_registry::publication::Draft =
        serde_json::from_value(changed).unwrap();
    assert!(
        registry
            .publish(
                &next,
                signed.record_bytes(),
                signed.envelope_bytes(),
                draft.predecessor(),
                &changed.sign(&key, &key, &key).unwrap()
            )
            .is_err()
    );
    assert_eq!(std::fs::read_dir(base.join("state")).unwrap().count(), 3);
}

#[test]
fn abandoned_reservations_burn_versions_without_publishing_orphan_views() {
    let (_temp, base, policy, key) = setup();
    std::fs::write(
        base.join(format!(
            "state/reservation-{}.json",
            &morphir_package::digest::Digest::of_bytes(b"[7,8,9]").to_string()[7..]
        )),
        b"[7,8,9]",
    )
    .unwrap();
    let first = library("1.0.0");
    let signed = first.sign(&key).unwrap();
    let registry = Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    let draft = registry
        .prepare(
            &first,
            signed.record_bytes(),
            signed.envelope_bytes(),
            "2098-01-01T00:00:00Z",
        )
        .unwrap();
    let value = serde_json::to_value(&draft).unwrap();
    assert_eq!(value["targets"]["version"], 8);
    assert_eq!(value["snapshot_version"], 9);
    assert_eq!(value["timestamp_version"], 10);
    assert_eq!(draft.predecessor(), &Predecessor::Absent);
    registry
        .publish(
            &first,
            signed.record_bytes(),
            signed.envelope_bytes(),
            draft.predecessor(),
            &draft.sign(&key, &key, &key).unwrap(),
        )
        .unwrap();
    assert!(base.join("registry/metadata/10.timestamp.json").exists());
}

#[test]
fn writer_process() {
    use std::io::{Read, Write};
    let Ok(base) = std::env::var("MORPHIR_PUBLICATION_TEST_BASE") else {
        return;
    };
    let base = std::path::PathBuf::from(base);
    let key = LocalSigningKey::from_seed([21; 32]);
    let version = std::env::var("MORPHIR_PUBLICATION_TEST_VERSION").unwrap();
    let library = library(&version);
    let signed = library.sign(&key).unwrap();
    let policy = std::fs::read(base.join("policy.json")).unwrap();
    let proposal: morphir_package::local_registry::publication::Proposal = serde_json::from_slice(
        &std::fs::read(base.join(format!("proposal-{version}.json"))).unwrap(),
    )
    .unwrap();
    let registry = Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    if std::env::var_os("MORPHIR_PUBLICATION_TEST_GATE").is_some() {
        std::io::stdout().write_all(b"writer-holds-lock\n").unwrap();
        std::io::stdout().flush().unwrap();
        std::io::stdin().read_exact(&mut [0]).unwrap();
    }
    let result = registry.publish(
        &library,
        signed.record_bytes(),
        signed.envelope_bytes(),
        &Predecessor::Absent,
        &proposal,
    );
    std::process::exit(match result {
        Ok(_) => 0,
        Err(morphir_package::local_registry::publication::Error::Conflict { .. }) => 74,
        Err(_) => 75,
    });
}
#[test]
fn two_process_writers_with_one_predecessor_have_exactly_one_winner() {
    use std::{
        io::{BufRead, Write},
        process::{Command, Stdio},
    };
    let (_temp, base, policy, key) = setup();
    std::fs::write(base.join("policy.json"), &policy).unwrap();
    {
        let registry =
            Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
        for version in ["1.0.0", "1.1.0"] {
            let library = library(version);
            let signed = library.sign(&key).unwrap();
            let draft = registry
                .prepare(
                    &library,
                    signed.record_bytes(),
                    signed.envelope_bytes(),
                    "2098-01-01T00:00:00Z",
                )
                .unwrap();
            std::fs::write(
                base.join(format!("proposal-{version}.json")),
                serde_json::to_vec(&draft.sign(&key, &key, &key).unwrap()).unwrap(),
            )
            .unwrap();
        }
    }
    let child = |version: &str| {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", "writer_process", "--nocapture"])
            .env("MORPHIR_PUBLICATION_TEST_BASE", &base)
            .env("MORPHIR_PUBLICATION_TEST_VERSION", version);
        cmd
    };
    let mut first = child("1.0.0")
        .env("MORPHIR_PUBLICATION_TEST_GATE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let output = first.stdout.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(output).lines() {
            if line.unwrap().contains("writer-holds-lock") {
                send.send(()).unwrap();
                break;
            }
        }
    });
    receive
        .recv_timeout(std::time::Duration::from_secs(15))
        .expect("first writer holds lock");
    let mut second = child("1.1.0").stdout(Stdio::null()).spawn().unwrap();
    first.stdin.take().unwrap().write_all(b"\n").unwrap();
    assert_eq!(first.wait().unwrap().code(), Some(0));
    assert_eq!(second.wait().unwrap().code(), Some(74));
    let current: Value = serde_json::from_slice(
        &std::fs::read(base.join("registry/metadata/timestamp.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(current["signed"]["version"], 1);
}

#[test]
fn predecessor_conflict_reports_normative_revision_witness() {
    let (_temp, base, policy, key) = setup();
    let library = library("1.0.0");
    let signed = library.sign(&key).unwrap();
    let registry = Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    let draft = registry
        .prepare(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            "2098-01-01T00:00:00Z",
        )
        .unwrap();
    let proposal = draft.sign(&key, &key, &key).unwrap();
    let result = registry
        .publish(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            draft.predecessor(),
            &proposal,
        )
        .unwrap();
    let error = registry
        .publish(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            &Predecessor::Absent,
            &proposal,
        )
        .unwrap_err();
    let wire = serde_json::to_value(error.diagnostic().unwrap()).unwrap();
    assert_eq!(wire["code"], "publication-conflict");
    assert_eq!(wire["phase"], "publication");
    assert_eq!(wire["category"], "domain-rejection");
    assert_eq!(
        wire["witnesses"][0],
        json!({"kind":"revision","expected":{"kind":"absent"},"actual":{"kind":"timestamp","digest":result.timestamp}})
    );
}

#[test]
fn idempotent_republication_preserves_authenticated_yank_status() {
    use morphir_package::local_registry::publication::{Draft, Proposal};
    let (_temp, base, policy, key) = setup();
    let library = library("1.0.0");
    let signed = library.sign(&key).unwrap();
    let old_timestamp;
    {
        let registry =
            Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
        let draft = registry
            .prepare(
                &library,
                signed.record_bytes(),
                signed.envelope_bytes(),
                "2098-01-01T00:00:00Z",
            )
            .unwrap();
        registry
            .publish(
                &library,
                signed.record_bytes(),
                signed.envelope_bytes(),
                draft.predecessor(),
                &draft.sign(&key, &key, &key).unwrap(),
            )
            .unwrap();
        let mut status_update = serde_json::to_value(&draft).unwrap();
        status_update["targets"]["version"] = json!(2);
        status_update["snapshot_version"] = json!(2);
        status_update["timestamp_version"] = json!(2);
        for pin in status_update["targets"]["targets"]
            .as_object_mut()
            .unwrap()
            .values_mut()
        {
            if pin["custom"]["morphir"]["kind"] == "LibraryRelease" {
                pin["custom"]["morphir"]["status"] = json!("yanked");
            }
        }
        let update: Draft = serde_json::from_value(status_update).unwrap();
        let proposal = update.sign(&key, &key, &key).unwrap();
        for (name, bytes) in [
            ("2.targets.json", &proposal.targets),
            ("2.snapshot.json", &proposal.snapshot),
            ("2.timestamp.json", &proposal.timestamp),
            ("timestamp.json", &proposal.timestamp),
        ] {
            std::fs::write(base.join("registry/metadata").join(name), bytes).unwrap();
        }
        old_timestamp = proposal.timestamp;
    }
    let registry = Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    let retry = registry
        .prepare(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            "2098-01-01T00:00:00Z",
        )
        .unwrap();
    let retry_wire = serde_json::to_value(&retry).unwrap();
    assert!(
        retry_wire["targets"]["targets"]
            .as_object()
            .unwrap()
            .values()
            .any(|pin| pin["custom"]["morphir"]["status"] == "yanked")
    );
    let invalid = Proposal {
        targets: vec![],
        snapshot: vec![],
        timestamp: vec![],
    };
    let result = registry
        .publish(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            &Predecessor::Timestamp {
                digest: morphir_package::digest::Digest::of_bytes(&old_timestamp).to_string(),
            },
            &invalid,
        )
        .unwrap();
    assert_eq!(result.outcome, Outcome::Idempotent);
    assert_eq!(
        std::fs::read(base.join("registry/metadata/timestamp.json")).unwrap(),
        old_timestamp
    );
    let targets: Value = serde_json::from_slice(
        &std::fs::read(base.join("registry/metadata/2.targets.json")).unwrap(),
    )
    .unwrap();
    assert!(
        targets["signed"]["targets"]
            .as_object()
            .unwrap()
            .values()
            .any(|pin| pin["custom"]["morphir"]["status"] == "yanked")
    );
    assert_eq!(std::fs::read_dir(base.join("state")).unwrap().count(), 3);
}

#[test]
fn replacing_a_lock_path_cannot_split_the_writer_lock_identity() {
    use std::{
        io::{BufRead, Write},
        process::{Command, Stdio},
    };
    let (_temp, base, policy, key) = setup();
    std::fs::write(base.join("policy.json"), &policy).unwrap();
    {
        let registry =
            Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
        for version in ["1.0.0", "1.1.0"] {
            let library = library(version);
            let signed = library.sign(&key).unwrap();
            let draft = registry
                .prepare(
                    &library,
                    signed.record_bytes(),
                    signed.envelope_bytes(),
                    "2098-01-01T00:00:00Z",
                )
                .unwrap();
            std::fs::write(
                base.join(format!("proposal-{version}.json")),
                serde_json::to_vec(&draft.sign(&key, &key, &key).unwrap()).unwrap(),
            )
            .unwrap();
        }
    }
    let spawn = |version: &str| {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "writer_process", "--nocapture"])
            .env("MORPHIR_PUBLICATION_TEST_BASE", &base)
            .env("MORPHIR_PUBLICATION_TEST_VERSION", version)
            .env("MORPHIR_PUBLICATION_TEST_GATE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let output = child.stdout.take().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(output).lines() {
                if line.unwrap().contains("writer-holds-lock") {
                    send.send(()).unwrap();
                    break;
                }
            }
        });
        (child, receive)
    };
    let (mut first, ready) = spawn("1.0.0");
    ready
        .recv_timeout(std::time::Duration::from_secs(15))
        .unwrap();
    let lock = base.join("registry/.publisher-lock");
    if lock.exists() {
        std::fs::remove_file(&lock).unwrap();
    }
    std::fs::write(lock, b"").unwrap();
    let (mut second, ready) = spawn("1.1.0");
    second.stdin.take().unwrap().write_all(b"\n").unwrap();
    let split = ready
        .recv_timeout(std::time::Duration::from_millis(500))
        .is_ok();
    first.stdin.take().unwrap().write_all(b"\n").unwrap();
    let first = first.wait().unwrap();
    let second = second.wait().unwrap();
    assert!(
        !split,
        "replacement lock pathname admitted a second writer while first still held the lock"
    );
    assert_eq!(first.code(), Some(0));
    assert_eq!(second.code(), Some(74));
}

#[test]
fn produced_consistent_snapshot_target_paths_fit_consumer_bounds() {
    fn check(path: &std::path::Path, root: &std::path::Path) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                check(&path, root);
            } else {
                let relative = path.strip_prefix(root).unwrap().to_str().unwrap();
                assert!(
                    morphir_package::local_registry::RegistryPath::parse(relative).is_ok(),
                    "{relative}"
                );
            }
        }
    }
    let (_temp, base, policy, key) = setup();
    let library = library("1.0.0");
    let signed = library.sign(&key).unwrap();
    let registry = Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    let draft = registry
        .prepare(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            "2098-01-01T00:00:00Z",
        )
        .unwrap();
    registry
        .publish(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            draft.predecessor(),
            &draft.sign(&key, &key, &key).unwrap(),
        )
        .unwrap();
    check(&base.join("registry/targets"), &base.join("registry"));
}

#[test]
fn committed_view_must_reconcile_signed_release_declarations_with_records() {
    use morphir_package::local_registry::publication::Draft;
    let (_temp, base, policy, key) = setup();
    let library = library("1.0.0");
    let signed = library.sign(&key).unwrap();
    {
        let registry =
            Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
        let draft = registry
            .prepare(
                &library,
                signed.record_bytes(),
                signed.envelope_bytes(),
                "2098-01-01T00:00:00Z",
            )
            .unwrap();
        registry
            .publish(
                &library,
                signed.record_bytes(),
                signed.envelope_bytes(),
                draft.predecessor(),
                &draft.sign(&key, &key, &key).unwrap(),
            )
            .unwrap();
        let mut changed = serde_json::to_value(&draft).unwrap();
        changed["targets"]["version"] = json!(2);
        changed["snapshot_version"] = json!(2);
        changed["timestamp_version"] = json!(2);
        for pin in changed["targets"]["targets"]
            .as_object_mut()
            .unwrap()
            .values_mut()
        {
            if pin["custom"]["morphir"]["kind"] == "LibraryRelease" {
                pin["custom"]["morphir"]["release"]["version"] = json!("9.9.9");
            }
        }
        let changed: Draft = serde_json::from_value(changed).unwrap();
        let proposal = changed.sign(&key, &key, &key).unwrap();
        for (name, bytes) in [
            ("2.targets.json", &proposal.targets),
            ("2.snapshot.json", &proposal.snapshot),
            ("2.timestamp.json", &proposal.timestamp),
            ("timestamp.json", &proposal.timestamp),
        ] {
            std::fs::write(base.join("registry/metadata").join(name), bytes).unwrap();
        }
    }
    assert!(Registry::open(&base.join("registry"), &base.join("state"), &policy).is_err());
}

#[test]
fn statement_target_carries_the_consumer_required_release_declaration() {
    let (_temp, base, policy, key) = setup();
    let library = library("1.0.0");
    let signed = library.sign(&key).unwrap();
    let registry = Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    let draft = registry
        .prepare(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            "2098-01-01T00:00:00Z",
        )
        .unwrap();
    let draft = serde_json::to_value(draft).unwrap();
    let statement = draft["targets"]["targets"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(path, _)| path.starts_with("statements/"))
        .unwrap()
        .1;
    assert_eq!(
        statement["custom"]["morphir"],
        json!({"formatVersion":"0.1.0-draft.3","kind":"LibraryReleaseStatement","release":{"packagePath":"example.com/greeting","version":"1.0.0"}})
    );
}

#[test]
fn role_versions_above_u64_are_exact_and_survive_restart() {
    use morphir_package::local_registry::publication::Draft;
    let (_temp, base, policy, key) = setup();
    let library = library("1.0.0");
    let signed = library.sign(&key).unwrap();
    {
        let registry =
            Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
        let draft = registry
            .prepare(
                &library,
                signed.record_bytes(),
                signed.envelope_bytes(),
                "2098-01-01T00:00:00Z",
            )
            .unwrap();
        let mut changed = serde_json::to_value(draft).unwrap();
        changed["targets"]["version"] = serde_json::from_str("18446744073709551616").unwrap();
        let draft: Draft = serde_json::from_value(changed).unwrap();
        registry
            .publish(
                &library,
                signed.record_bytes(),
                signed.envelope_bytes(),
                draft.predecessor(),
                &draft.sign(&key, &key, &key).unwrap(),
            )
            .unwrap();
    }
    let registry = Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    let draft = registry
        .prepare(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            "2098-01-01T00:00:00Z",
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(draft).unwrap()["targets"]["version"].to_string(),
        "18446744073709551617"
    );
}

#[test]
fn relative_initialization_uses_the_process_working_directory() {
    let (_temp, base, policy, _key) = setup();
    std::fs::write(base.join("policy.json"), policy).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "relative_initialization_process", "--nocapture"])
        .current_dir(&base)
        .env("MORPHIR_TEST_RELATIVE_INITIALIZATION", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        base.join("relative-registry/metadata/1.root.json")
            .is_file()
    );
    assert!(base.join("relative-state/identity.json").is_file());
}
#[test]
fn relative_initialization_process() {
    if std::env::var_os("MORPHIR_TEST_RELATIVE_INITIALIZATION").is_none() {
        return;
    }
    let policy = std::fs::read("policy.json").unwrap();
    let root = std::fs::read("registry/metadata/1.root.json").unwrap();
    let registry = std::path::Path::new("relative-registry");
    let state = std::path::Path::new("relative-state");
    Registry::initialize(registry, state, &policy, &root).unwrap();
    Registry::open(registry, state, &policy).unwrap();
}

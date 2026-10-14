use morphir_package::{
    digest::Digest,
    local_registry::{
        decode_library_lock,
        mvp::{self, InitializeRequest, RestoreRequest},
    },
};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[path = "local_registry/snapshot_mothers.rs"]
mod signed_provider;
#[path = "local_registry/tuf_mothers.rs"]
#[allow(dead_code)]
mod tuf_mothers;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/local_registry/mvp-fixture")
}

fn copy_tree(source: &Path, destination: &Path) {
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

struct RegistryMother {
    directory: tempfile::TempDir,
    policy: Vec<u8>,
    lock: Vec<u8>,
}

impl RegistryMother {
    fn a_fresh_registry() -> Self {
        let directory = tempfile::tempdir().unwrap();
        copy_tree(
            &fixture().join("registry"),
            &directory.path().join("registry"),
        );
        let policy = fs::read_to_string(fixture().join("trust-policy.json"))
            .unwrap()
            .replace("previous-authorization", "fresh-metadata")
            .into_bytes();
        let lock = fs::read(fixture().join("morphir.lock")).unwrap();
        mvp::initialize(InitializeRequest {
            policy: &policy,
            root: &fs::read(fixture().join("registry/metadata/1.root.json")).unwrap(),
            state: &directory.path().join("trust"),
        })
        .unwrap();
        Self {
            directory,
            policy,
            lock,
        }
    }

    async fn snapshot(&self, name: &str) -> Result<mvp::RestoredSnapshot, mvp::Error> {
        mvp::restore_snapshot(RestoreRequest {
            policy: &self.policy,
            lock: &self.lock,
            registry: &self.directory.path().join("registry"),
            state: &self.directory.path().join("trust"),
            output: &self.directory.path().join(name),
        })
        .await
    }
}

#[tokio::test]
async fn snapshot_retains_exact_verified_bytes_after_files_are_removed() {
    let registry = RegistryMother::a_fresh_registry();
    let snapshot = registry.snapshot("out").await.unwrap();
    let lock = decode_library_lock(&registry.lock).unwrap();
    assert_eq!(snapshot.root(), lock.graph().root());
    assert_eq!(
        snapshot.policy_digest(),
        &Digest::of_bytes(&registry.policy)
    );
    assert_eq!(snapshot.lock_digest(), &Digest::of_bytes(&registry.lock));
    assert_eq!(snapshot.libraries().len(), 2);
    assert!(!registry.directory.path().join("trust/operation").exists());

    for library in snapshot.libraries() {
        let acquisition = lock
            .acquisitions()
            .iter()
            .find(|a| a.release() == library.release())
            .unwrap();
        let source = registry
            .directory
            .path()
            .join("registry")
            .join(acquisition.source().path().as_str());
        let manifest = fs::read(source.join("manifest.json")).unwrap();
        let payload = fs::read(source.join("ir.json")).unwrap();
        assert_eq!(library.manifest(), manifest);
        assert_eq!(library.payload().path().as_str(), "ir.json");
        assert_eq!(library.payload().bytes(), payload);
        let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
        assert_eq!(
            library.payload().digest().to_string(),
            manifest["content"]["ir.json"]
        );
        assert!(library.context_resources().is_empty());
    }

    let report = mvp::restore(RestoreRequest {
        policy: &registry.policy,
        lock: &registry.lock,
        registry: &registry.directory.path().join("registry"),
        state: &registry.directory.path().join("trust"),
        output: &registry.directory.path().join("legacy"),
    })
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(snapshot.report()).unwrap(),
        serde_json::to_value(report).unwrap()
    );
    fs::remove_dir_all(registry.directory.path().join("out")).unwrap();
    fs::remove_dir_all(registry.directory.path().join("registry")).unwrap();
    assert!(
        snapshot
            .libraries()
            .iter()
            .all(|l| !l.payload().bytes().is_empty())
    );
    let debug = format!("{snapshot:?}");
    assert!(
        !debug.contains("\"distribution\""),
        "Debug must not dump payloads"
    );
}

#[tokio::test]
async fn an_old_snapshot_does_not_bypass_fresh_authentication() {
    let registry = RegistryMother::a_fresh_registry();
    let previous = registry.snapshot("first").await.unwrap();
    fs::write(
        registry
            .directory
            .path()
            .join("registry/metadata/timestamp.json"),
        b"{}",
    )
    .unwrap();
    assert!(registry.snapshot("second").await.is_err());
    assert!(!registry.directory.path().join("second").exists());
    assert_eq!(previous.libraries().len(), 2);
}

#[tokio::test]
async fn a_tampered_dependency_prevents_the_entire_snapshot() {
    let registry = RegistryMother::a_fresh_registry();
    let lock = decode_library_lock(&registry.lock).unwrap();
    let dependency = lock
        .acquisitions()
        .iter()
        .find(|a| a.release() != lock.graph().root())
        .unwrap();
    let payload = registry
        .directory
        .path()
        .join("registry")
        .join(dependency.source().path().as_str())
        .join("ir.json");
    fs::write(payload, b"tampered dependency").unwrap();
    let error = registry.snapshot("out").await.unwrap_err();
    assert!(
        error.to_string().contains("content digest mismatch"),
        "{error}"
    );
    assert!(!registry.directory.path().join("out").exists());
}

#[tokio::test]
async fn a_failed_state_write_returns_no_snapshot_or_publication() {
    let registry = RegistryMother::a_fresh_registry();
    let db =
        rusqlite::Connection::open(registry.directory.path().join("trust/trust.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_write BEFORE UPDATE ON state BEGIN SELECT RAISE(FAIL, 'snapshot test write failure'); END;").unwrap();
    drop(db);
    let error = registry.snapshot("out").await.unwrap_err();
    assert!(
        error.to_string().contains("snapshot test write failure"),
        "{error}"
    );
    assert!(!registry.directory.path().join("out").exists());
    assert!(registry.directory.path().join("trust/operation").exists());
}

#[tokio::test]
async fn signed_native_metadata_and_contexts_survive_without_filesystem_reads() {
    use morphir_core::{
        data_value::DataValueValidator,
        ir::{json::read_ir_file, v4},
        metadata::{
            Carrier, ContextResources, Fact, GraphName, ObjectTerm,
            admission::{Admission, PredicateClosure, SubjectRole},
            inline_document_contexts,
        },
        node_address::NodeUri,
    };
    let provider = signed_provider::a_signed_provider().await;
    let snapshot = mvp::restore_snapshot(RestoreRequest {
        policy: &provider.policy,
        lock: &provider.lock,
        registry: &provider.directory.path().join("registry"),
        state: &provider.directory.path().join("trust"),
        output: &provider.directory.path().join("out"),
    })
    .await
    .unwrap();
    assert_eq!(snapshot.libraries().len(), 1);
    let library = &snapshot.libraries()[0];
    assert_eq!(
        library.release().package_path().as_str(),
        "example.com/metadata/provider"
    );
    assert_eq!(library.manifest(), provider.manifest);
    assert_eq!(library.payload().bytes(), provider.native);
    assert_eq!(library.context_resources().len(), 1);
    let context = &library.context_resources()[0];
    assert_eq!(context.path().as_str(), "contexts/schema.jsonld");
    assert_eq!(context.bytes(), signed_provider::CONTEXT);
    assert_eq!(
        context.digest(),
        &Digest::of_bytes(signed_provider::CONTEXT)
    );
    fs::remove_dir_all(provider.directory.path().join("registry")).unwrap();
    fs::remove_dir_all(provider.directory.path().join("out")).unwrap();

    let mut resources = ContextResources::new(".");
    resources.insert_local(context.path().as_str(), context.bytes().to_vec());
    let native = serde_json::from_slice(library.payload().bytes()).unwrap();
    let inline = inline_document_contexts(&native, &resources, Some("ir.json")).unwrap();
    let (file, warnings): (v4::IRFile, _) = read_ir_file(&inline.to_string()).unwrap();
    assert!(warnings.is_empty());
    let closure = PredicateClosure::from_v4_provider(&file, &resources).unwrap();
    let validator = DataValueValidator::v4(&file.distribution).unwrap();
    let fact = Fact::new(
        NodeUri::parse("morphir://ir/pkg/acme/orders?format=4.1.0#/module/api/value/submit-order")
            .unwrap(),
        NodeUri::parse(signed_provider::PREDICATE).unwrap(),
        ObjectTerm::value(serde_json::json!(true)),
        GraphName::Default,
    );
    assert_eq!(
        closure.admit(
            &fact,
            SubjectRole::ValueSpecification,
            &Carrier::DocumentGraph,
            &validator,
            &[],
            None
        ),
        Ok(Admission::Validated)
    );
}

#[tokio::test]
async fn changed_authenticated_context_refuses_the_snapshot() {
    let provider = signed_provider::a_signed_provider().await;
    fs::write(
        provider
            .directory
            .path()
            .join("registry")
            .join(provider.bundle)
            .join("contexts/schema.jsonld"),
        b"{}",
    )
    .unwrap();
    let result = mvp::restore_snapshot(RestoreRequest {
        policy: &provider.policy,
        lock: &provider.lock,
        registry: &provider.directory.path().join("registry"),
        state: &provider.directory.path().join("trust"),
        output: &provider.directory.path().join("out"),
    })
    .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("content digest mismatch")
    );
    assert!(!provider.directory.path().join("out").exists());
}

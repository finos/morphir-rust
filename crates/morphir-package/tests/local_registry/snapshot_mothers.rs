//! Signed provider fixture with independent TUF signing and literal native facts.
use super::tuf_mothers as tuf;
use morphir_package::{
    authoring::{AuthoredLibrary, LocalSigningKey},
    digest::Digest,
    local_registry::mvp::{self, InitializeRequest, ResolveRequest},
    resolution::{PackagePath, ReleaseId, StableVersion},
};
use serde_json::json;
use std::{fs, path::Path};

pub const CONTEXT: &[u8] = br#"{
  "@context": {"@vocab": "morphir://ir/pkg/morphir/metadata?format=4.1.0#/module/schema/value/"}
}
"#;
pub const PREDICATE: &str =
    "morphir://ir/pkg/acme/metadata?format=4.1.0#/module/lifecycle/value/deprecated";

pub struct SignedProvider {
    pub directory: tempfile::TempDir,
    pub policy: Vec<u8>,
    pub lock: Vec<u8>,
    pub native: Vec<u8>,
    pub manifest: Vec<u8>,
    pub bundle: String,
}

fn write(root: &Path, path: &str, bytes: &[u8]) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn target(root: &Path, path: &str, bytes: &[u8]) -> serde_json::Value {
    let (parent, name) = path.rsplit_once('/').unwrap();
    let digest = Digest::of_bytes(bytes).to_string();
    let hash = digest.strip_prefix("sha256:").unwrap();
    write(root, &format!("targets/{parent}/{hash}.{name}"), bytes);
    json!({"length": bytes.len(), "hashes": {"sha256": hash}})
}

pub async fn a_signed_provider() -> SignedProvider {
    let directory = tempfile::tempdir().unwrap();
    let registry = directory.path().join("registry");
    let native = serde_json::to_vec_pretty(&json!({
        "formatVersion": "4.1.0",
        "distribution": {"Library": {
            "packageName": "acme/metadata", "dependencies": {},
            "def": {"modules": {"lifecycle": {"Public": {
                "types": {}, "values": {"deprecated": {"Public": {"ExpressionBody": {
                    "inputTypes": {}, "outputType": "morphir/SDK:basics#bool",
                    "body": {"Literal": {"BoolLiteral": true}}
                }}}}
            }}}}
        }},
        "$meta": {"@context": "./contexts/schema.jsonld", "@graph": [{
            "@id": PREDICATE,
            "subject-role": ["ValueSpecification", "ValueDefinition"],
            "object-form": "data", "interpreter": "descriptive"
        }]}
    }))
    .unwrap();
    let library = AuthoredLibrary::create_with_contexts(
        &serde_json::to_vec(&json!({
            "packagePath": "example.com/metadata/provider", "version": "1.0.0",
            "dependencies": {}, "exports": {"lifecycle": "lifecycle"}
        }))
        .unwrap(),
        &native,
        vec![("contexts/schema.jsonld".into(), CONTEXT.to_vec())],
    )
    .unwrap();
    let publisher = LocalSigningKey::from_seed([21; 32]);
    let signed = library.sign(&publisher).unwrap();
    let record: serde_json::Value = serde_json::from_slice(signed.record_bytes()).unwrap();
    let statement_path = record["statement"]["path"].as_str().unwrap();
    let release = record["release"].clone();
    let bundle = record["source"]["path"].as_str().unwrap().to_owned();
    write(
        &registry,
        &format!("{bundle}/manifest.json"),
        library.manifest_bytes(),
    );
    write(&registry, &format!("{bundle}/ir.json"), library.ir_bytes());
    write(
        &registry,
        &format!("{bundle}/contexts/schema.jsonld"),
        CONTEXT,
    );

    let mut targets = tuf::targets_body(1);
    let record_path = "records/provider-1.0.0.json";
    let mut record_target = target(&registry, record_path, signed.record_bytes());
    record_target["custom"] = json!({"morphir": {
        "formatVersion": "0.1.0-draft.3", "kind": "LibraryRelease",
        "release": release, "status": "active"
    }});
    targets["targets"][record_path] = record_target;
    let mut statement_target = target(&registry, statement_path, signed.envelope_bytes());
    statement_target["custom"] = json!({"morphir": {
        "formatVersion": "0.1.0-draft.3", "kind": "LibraryReleaseStatement",
        "release": release
    }});
    targets["targets"][statement_path] = statement_target;
    let targets = tuf::sign(targets, &[(42, tuf::key(42))]);
    let root = tuf::root(1, 42);
    let snapshot = tuf::snapshot(&targets, 42);
    let timestamp = tuf::timestamp(&snapshot, 42);
    for (path, bytes) in [
        ("metadata/1.root.json", &root),
        ("metadata/1.targets.json", &targets),
        ("metadata/1.snapshot.json", &snapshot),
        ("metadata/1.timestamp.json", &timestamp),
        ("metadata/timestamp.json", &timestamp),
    ] {
        write(&registry, path, bytes);
    }
    let root_digest = Digest::of_bytes(&root).to_string();
    let policy = serde_json::to_vec(&json!({
        "formatVersion": "0.1.0-draft.3", "kind": "LibraryTrustPolicy",
        "repositories": [{"identity": root_digest,
            "bootstrapRoot": {"version": 1, "digest": root_digest},
            "namespaces": ["example.com"]}],
        "publisherRules": [{"namespace": "example.com",
            "publicKeys": [publisher.public_key_hex()], "threshold": 1}],
        "continuedUse": "fresh-metadata"
    }))
    .unwrap();
    mvp::initialize(InitializeRequest {
        policy: &policy,
        root: &root,
        state: &directory.path().join("trust"),
    })
    .unwrap();
    mvp::resolve(ResolveRequest {
        policy: &policy,
        root: ReleaseId::new(
            PackagePath::parse("example.com/metadata/provider").unwrap(),
            StableVersion::parse("1.0.0").unwrap(),
        ),
        registry: &registry,
        state: &directory.path().join("trust"),
        output: &directory.path().join("morphir.lock"),
    })
    .await
    .unwrap();
    let lock = fs::read(directory.path().join("morphir.lock")).unwrap();
    SignedProvider {
        directory,
        policy,
        lock,
        native,
        manifest: library.manifest_bytes().to_vec(),
        bundle,
    }
}

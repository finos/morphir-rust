use super::tuf_mothers;
use morphir_package::{
    local_registry::mvp::{self, InitializeRequest},
    resolution::{PackagePath, ReleaseId, StableVersion},
};
use std::{fs, path::PathBuf};

pub fn fixture() -> PathBuf {
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
pub fn registry_with_targets(
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

pub fn root() -> ReleaseId {
    ReleaseId::new(
        PackagePath::parse("example.com/finance/loan-rules").unwrap(),
        StableVersion::parse("1.0.0").unwrap(),
    )
}

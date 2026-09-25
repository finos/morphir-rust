use morphir_common::ir_transport::metadata::{
    ContextDigestResolver, ContextRequest, ContextResourceError, ContextResourceLimits,
    ResolvedContextResource, load_context_resources,
};
use morphir_core::metadata::{ContextError, resolve_context};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

const ALIAS: &str = "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/value/aliases";

fn context(value: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"@context": value})).unwrap()
}

fn digest_reference(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("morphir://context/sha256/{digest}")
}

fn limits() -> ContextResourceLimits {
    ContextResourceLimits::new(1024, 4096, 8, 8)
}

fn load(
    root: &Path,
    authored: &Value,
) -> Result<morphir_core::metadata::ContextResources, ContextResourceError> {
    load_context_resources(root, &[ContextRequest::at_root(authored)], None, limits())
}

struct Resolver(BTreeMap<String, ResolvedContextResource>);

impl ContextDigestResolver for Resolver {
    fn resolve(
        &self,
        reference: &str,
        _max_bytes: usize,
    ) -> Result<Option<ResolvedContextResource>, String> {
        Ok(self.0.get(reference).cloned())
    }
}

#[test]
fn nested_local_import_is_loaded_from_its_containing_file() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("contexts")).unwrap();
    std::fs::write(
        root.path().join("contexts/main.jsonld"),
        context(json!("./leaf.jsonld")),
    )
    .unwrap();
    std::fs::write(
        root.path().join("contexts/leaf.jsonld"),
        context(json!({"alias": ALIAS})),
    )
    .unwrap();
    let authored = json!("contexts/main.jsonld");
    let resources = load(root.path(), &authored).unwrap();
    let effective = resolve_context(None, &authored, &resources, None).unwrap();
    assert_eq!(
        effective.expand_key("alias").unwrap().uri().to_string(),
        ALIAS
    );
}

#[test]
fn document_tree_context_uses_the_document_file_as_its_base() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("documents")).unwrap();
    std::fs::write(
        root.path().join("documents/terms.jsonld"),
        context(json!({"alias": ALIAS})),
    )
    .unwrap();
    let authored = json!("terms.jsonld");
    let request = ContextRequest::in_file(&authored, "documents/model.json");
    let resources = load_context_resources(root.path(), &[request], None, limits()).unwrap();
    assert_eq!(
        resolve_context(None, &authored, &resources, request.base_file)
            .unwrap()
            .expand_key("alias")
            .unwrap()
            .uri()
            .to_string(),
        ALIAS
    );
}

#[test]
fn configured_resource_count_and_depth_are_enforced() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("one.jsonld"), context(json!("two.jsonld"))).unwrap();
    std::fs::write(root.path().join("two.jsonld"), context(json!({}))).unwrap();
    let authored = json!("one.jsonld");
    assert!(matches!(
        load_context_resources(
            root.path(),
            &[ContextRequest::at_root(&authored)],
            None,
            ContextResourceLimits::new(1024, 4096, 1, 8)
        ),
        Err(ContextResourceError::ResourceCountExceeded)
    ));
    assert!(matches!(
        load_context_resources(
            root.path(),
            &[ContextRequest::at_root(&authored)],
            None,
            ContextResourceLimits::new(1024, 4096, 8, 1)
        ),
        Err(ContextResourceError::ImportDepthExceeded)
    ));
}

#[test]
fn lexical_escape_and_missing_or_directory_resources_fail() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("folder.jsonld")).unwrap();
    assert!(matches!(
        load(root.path(), &json!("../escape.jsonld")),
        Err(ContextResourceError::PathEscape)
    ));
    assert!(matches!(
        load(root.path(), &json!("missing.jsonld")),
        Err(ContextResourceError::Missing(_))
    ));
    assert!(matches!(
        load(root.path(), &json!("folder.jsonld")),
        Err(ContextResourceError::NotFile(_))
    ));
}

#[cfg(unix)]
#[test]
fn symlink_outside_root_is_rejected() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("leaf.jsonld"), context(json!({}))).unwrap();
    symlink(outside.path(), root.path().join("contexts")).unwrap();
    assert!(matches!(
        load(root.path(), &json!("contexts/leaf.jsonld")),
        Err(ContextResourceError::PathEscape)
    ));
}

#[test]
fn per_resource_and_total_byte_budgets_are_enforced() {
    let root = tempfile::tempdir().unwrap();
    let bytes = context(json!({"alias": ALIAS}));
    std::fs::write(root.path().join("one.jsonld"), &bytes).unwrap();
    std::fs::write(root.path().join("two.jsonld"), &bytes).unwrap();
    let single = ContextResourceLimits::new(bytes.len() - 1, 4096, 8, 8);
    assert!(matches!(
        load_context_resources(
            root.path(),
            &[ContextRequest::at_root(&json!("one.jsonld"))],
            None,
            single
        ),
        Err(ContextResourceError::ResourceBytesExceeded(_))
    ));
    let total = ContextResourceLimits::new(1024, bytes.len() * 2 - 1, 8, 8);
    assert!(matches!(
        load_context_resources(
            root.path(),
            &[ContextRequest::at_root(&json!([
                "one.jsonld",
                "two.jsonld"
            ]))],
            None,
            total
        ),
        Err(ContextResourceError::TotalBytesExceeded)
    ));
}

#[test]
fn core_reports_duplicate_and_cycle_from_loaded_closure() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("one.jsonld"), context(json!("two.jsonld"))).unwrap();
    std::fs::write(root.path().join("two.jsonld"), context(json!("one.jsonld"))).unwrap();
    assert!(matches!(
        load(root.path(), &json!("one.jsonld")),
        Err(ContextResourceError::Context(ContextError::ImportCycle(_)))
    ));
    std::fs::write(root.path().join("two.jsonld"), context(json!({}))).unwrap();
    assert!(matches!(
        load(root.path(), &json!(["one.jsonld", "two.jsonld"])),
        Err(ContextResourceError::Context(
            ContextError::DuplicateImport(_)
        ))
    ));
}

#[test]
fn trusted_digest_is_checked_against_raw_bytes() {
    let root = tempfile::tempdir().unwrap();
    let bytes = context(json!({"alias": ALIAS}));
    let reference = digest_reference(&bytes);
    let resolver = Resolver(BTreeMap::from([(
        reference.clone(),
        ResolvedContextResource::trusted(bytes.clone()),
    )]));
    let authored = json!(reference);
    let resources = load_context_resources(
        root.path(),
        &[ContextRequest::at_root(&authored)],
        Some(&resolver),
        limits(),
    )
    .unwrap();
    assert_eq!(
        resolve_context(None, &authored, &resources, None)
            .unwrap()
            .expand_key("alias")
            .unwrap()
            .uri()
            .to_string(),
        ALIAS
    );
    let wrong = format!("morphir://context/sha256/{}", "0".repeat(64));
    let resolver = Resolver(BTreeMap::from([(
        wrong.clone(),
        ResolvedContextResource::trusted(bytes),
    )]));
    assert!(matches!(
        load_context_resources(
            root.path(),
            &[ContextRequest::at_root(&json!(wrong))],
            Some(&resolver),
            limits()
        ),
        Err(ContextResourceError::Context(ContextError::DigestMismatch(
            _
        )))
    ));
}

#[test]
fn untrusted_digest_and_content_addressed_relative_child_fail() {
    let root = tempfile::tempdir().unwrap();
    let bytes = context(json!({}));
    let reference = digest_reference(&bytes);
    let resolver = Resolver(BTreeMap::from([(
        reference.clone(),
        ResolvedContextResource::untrusted(),
    )]));
    assert!(matches!(
        load_context_resources(
            root.path(),
            &[ContextRequest::at_root(&json!(reference))],
            Some(&resolver),
            limits()
        ),
        Err(ContextResourceError::Context(
            ContextError::ResourceUntrusted(_)
        ))
    ));
    let bytes = context(json!("child.jsonld"));
    let reference = digest_reference(&bytes);
    let resolver = Resolver(BTreeMap::from([(
        reference.clone(),
        ResolvedContextResource::trusted(bytes),
    )]));
    assert!(matches!(
        load_context_resources(
            root.path(),
            &[ContextRequest::at_root(&json!(reference))],
            Some(&resolver),
            limits()
        ),
        Err(ContextResourceError::Context(
            ContextError::RelativeImportWithoutBase
        ))
    ));
}

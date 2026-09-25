use std::fs;

use morphir_common::ir_transport::metadata::{
    ContextExportRequest, ContextOutput, ContextResourceLimits, ContextStorage, export_contexts,
};
use morphir_core::metadata::{ContextResources, resolve_context};
use serde_json::{Value, json};

const PREDICATE: &str =
    "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/value/operational-name";
const STEM: &str = "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/value/";

fn authoring_document() -> Value {
    json!({
        "formatVersion": "4.1.0",
        "$meta": {
            "@context": "contexts/main.jsonld",
            "@graph": [{"@id": "morphir://ir/pkg/acme/model?format=4.1.0#/module/api/value/item", "ops:operational-name": "Submit"}]
        },
        "distribution": {
            "attributes": {
                "@context": {"localName": "ops:operational-name"},
                "facts": {"localName": "Submit", "arbitrary": {"@type": "@json", "@value": {"attributes": {"@context": "do-not-touch"}}}}
            }
        }
    })
}

fn authoring_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("contexts")).unwrap();
    fs::write(
        root.path().join("contexts/main.jsonld"),
        br#"{"@context":["./terms.jsonld",{"@vocab":"morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/value/"}]}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("contexts/terms.jsonld"),
        format!(r#"{{"@context":{{"ops":{{"@id":"{STEM}","@prefix":true,"@protected":true}}}}}}"#),
    )
    .unwrap();
    root
}

#[test]
fn standalone_auto_materializes_imports_inline_and_keeps_scopes() {
    let root = authoring_root();
    let document = authoring_document();
    let exported = export_contexts(ContextExportRequest {
        source_root: root.path(),
        document: &document,
        source_file: Some("ir.json"),
        output_file: None,
        output: ContextOutput::Standalone,
        storage: ContextStorage::Auto,
        resolver: None,
        limits: ContextResourceLimits::default(),
    })
    .unwrap();
    assert!(exported.resources.is_empty());
    assert_eq!(
        exported.document["$meta"]["@context"]["ops"]["@protected"],
        true
    );
    assert_eq!(
        exported.document["distribution"]["attributes"]["@context"]["localName"],
        PREDICATE
    );
    assert_eq!(
        exported.document["distribution"]["attributes"]["facts"]["arbitrary"]["@value"]["attributes"]
            ["@context"],
        "do-not-touch"
    );
    drop(root);
    let empty = ContextResources::new(".");
    let document_scope =
        resolve_context(None, &exported.document["$meta"]["@context"], &empty, None).unwrap();
    let attribute_scope = resolve_context(
        Some(&document_scope),
        &exported.document["distribution"]["attributes"]["@context"],
        &empty,
        None,
    )
    .unwrap();
    assert_eq!(
        attribute_scope
            .expand_key("localName")
            .unwrap()
            .uri()
            .to_string(),
        PREDICATE
    );
}

#[test]
fn tree_auto_exports_digest_inventory_and_rereads_without_authoring_workspace() {
    let root = authoring_root();
    let authored = authoring_document();
    let exported = export_contexts(ContextExportRequest {
        source_root: root.path(),
        document: &authored,
        source_file: Some("ir.json"),
        output_file: Some("documents/ir.json"),
        output: ContextOutput::DocumentTree,
        storage: ContextStorage::Auto,
        resolver: None,
        limits: ContextResourceLimits::default(),
    })
    .unwrap();
    assert_eq!(exported.resources.len(), 2);
    assert!(exported.resources.iter().all(|file| {
        file.path.starts_with("contexts/")
            && file.path.ends_with(".jsonld")
            && file.media_type == "application/ld+json"
            && file.sha256.len() == 64
    }));
    assert!(
        exported.document["$meta"]["@context"]
            .as_str()
            .unwrap()
            .starts_with("../contexts/")
    );
    let output = tempfile::tempdir().unwrap();
    fs::create_dir(output.path().join("documents")).unwrap();
    fs::write(
        output.path().join("documents/ir.json"),
        serde_json::to_vec(&exported.document).unwrap(),
    )
    .unwrap();
    for file in &exported.resources {
        let path = output.path().join(&file.path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, &file.bytes).unwrap();
    }
    drop(root);
    let reread = export_contexts(ContextExportRequest {
        source_root: output.path(),
        document: &exported.document,
        source_file: Some("documents/ir.json"),
        output_file: None,
        output: ContextOutput::Standalone,
        storage: ContextStorage::Inline,
        resolver: None,
        limits: ContextResourceLimits::default(),
    })
    .unwrap();
    assert!(reread.resources.is_empty());
    assert_eq!(
        reread.document["distribution"]["attributes"]["@context"]["localName"],
        PREDICATE
    );
}

#[test]
fn external_policy_requires_a_tree_destination() {
    let root = authoring_root();
    let authored = authoring_document();
    let error = export_contexts(ContextExportRequest {
        source_root: root.path(),
        document: &authored,
        source_file: Some("ir.json"),
        output_file: None,
        output: ContextOutput::Standalone,
        storage: ContextStorage::External,
        resolver: None,
        limits: ContextResourceLimits::default(),
    })
    .unwrap_err();
    assert!(error.to_string().contains("document-tree destination"));
}

#[test]
fn external_export_rejects_a_resource_that_cannot_be_reread_under_the_same_limit() {
    let source = tempfile::tempdir().unwrap();
    let document = json!({
        "formatVersion": "4.1.0",
        "$meta": {"@context": {"operationalName": PREDICATE}}
    });
    let error = export_contexts(ContextExportRequest {
        source_root: source.path(),
        document: &document,
        source_file: None,
        output_file: Some("ir.json"),
        output: ContextOutput::Archive,
        storage: ContextStorage::Auto,
        resolver: None,
        limits: ContextResourceLimits::new(16, 1024, 10, 10),
    })
    .unwrap_err();
    assert!(error.to_string().contains("byte limit"));
}

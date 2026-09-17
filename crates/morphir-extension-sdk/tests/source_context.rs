use morphir_extension_sdk::{CompileRequest, SourceDocument};
use serde_json::json;

fn request(root: serde_json::Value, uris: &[&str]) -> CompileRequest {
    serde_json::from_value(json!({
        "languageId": "python", "package": {"name": "acme/example"},
        "documents": uris.iter().map(|uri| SourceDocument {
            uri: (*uri).into(), language_id: "python".into(), version: 1, text: String::new(),
        }).collect::<Vec<_>>(),
        "options": {"typesOnly": false, "irVersion": "4", "sourceRootUri": root},
    }))
    .unwrap()
}

#[test]
fn source_context_preserves_relative_identity_and_ignores_revision_metadata() {
    let request = request(
        json!("file:///project/src?revision=1"),
        &[
            "file:///project/src/domain/%6dodels.py?revision=2#selection",
            "domain/rules.py",
        ],
    );
    let paths = request.source_paths().unwrap();
    assert_eq!(
        paths.iter().map(|path| path.as_str()).collect::<Vec<_>>(),
        ["domain/models.py", "domain/rules.py"]
    );
    assert!(request.package.exposed_modules.is_none());
}

#[test]
fn invalid_roots_escape_paths_and_duplicate_document_identities_are_rejected() {
    for input in [
        request(json!(123), &["models.py"]),
        request(json!("src"), &["models.py"]),
        request(
            json!("file:///project/src"),
            &["file:///other/src/models.py"],
        ),
        request(json!("file:///project/src"), &["domain/%2e%2e/models.py"]),
        request(json!("file:///project/src"), &["domain%2fmodels.py"]),
        request(json!("file:///project/src"), &["domain/%GGmodels.py"]),
        request(
            json!("file:///project/src"),
            &["models.py", "file:///project/src/models.py?rev=1"],
        ),
    ] {
        assert!(input.source_paths().is_err(), "{input:?}");
    }
}

#[test]
fn hierarchical_roots_and_native_paths_preserve_nested_identity() {
    for (root, document, expected) in [
        ("file:///", "file:///domain/models.py", "domain/models.py"),
        (
            "file:///C:/project/src",
            "file:///C:/project/src/domain/models.py",
            "domain/models.py",
        ),
        (
            r"C:\project\src",
            r"C:\project\src\domain\models.py",
            "domain/models.py",
        ),
        (
            "https://example.org/sources",
            "https://example.org/sources/domain/models.py",
            "domain/models.py",
        ),
    ] {
        assert_eq!(
            request(json!(root), &[document]).source_paths().unwrap()[0].as_str(),
            expected
        );
    }
    for document in [
        "file:///project/src-other/models.py",
        "file://other/project/src/models.py",
    ] {
        assert!(
            request(json!("file:///project/src"), &[document])
                .source_paths()
                .is_err()
        );
    }
}

#[test]
fn multiple_absolute_documents_require_a_root_but_single_document_compatibility_remains() {
    let mut input = request(
        json!("file:///project/src"),
        &["file:///project/src/domain/models.py"],
    );
    input.options.extra.remove("sourceRootUri");
    assert_eq!(input.source_paths().unwrap()[0].as_str(), "models.py");
    input.documents.push(SourceDocument {
        uri: "file:///project/src/rules.py".into(),
        ..input.documents[0].clone()
    });
    assert!(input.source_paths().is_err());
}

#[test]
fn exposure_distinguishes_omission_from_an_empty_public_module_list() {
    let mut input = request(json!("file:///project/src"), &["models.py"]);
    assert!(
        serde_json::to_value(&input).unwrap()["package"]
            .get("exposedModules")
            .is_none()
    );
    input.package.exposed_modules = Some(vec![]);
    assert_eq!(
        serde_json::to_value(&input).unwrap()["package"]["exposedModules"],
        json!([])
    );
}

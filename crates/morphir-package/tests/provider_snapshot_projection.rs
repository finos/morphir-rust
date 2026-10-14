use morphir_package::{
    local_registry::mvp::{self, RestoreRequest},
    provider_snapshot::{ProjectionError, project_library},
};
use serde_json::json;

#[path = "local_registry/snapshot_mothers.rs"]
#[allow(dead_code)]
mod mothers;
#[path = "local_registry/tuf_mothers.rs"]
#[allow(dead_code)]
mod tuf_mothers;

#[tokio::test]
async fn checked_projection_keeps_native_revision_and_document_metadata_separate() {
    let provider = mothers::a_signed_provider().await;
    let snapshot = mvp::restore_snapshot(RestoreRequest {
        policy: &provider.policy,
        lock: &provider.lock,
        registry: &provider.directory.path().join("registry"),
        state: &provider.directory.path().join("trust"),
        output: &provider.directory.path().join("out"),
    })
    .await
    .unwrap();
    let library = &snapshot.libraries()[0];
    let projected = project_library(library).unwrap();
    assert_eq!(projected.native_digest(), library.payload().digest());
    assert_eq!(library.payload().bytes(), provider.native);
    let core: serde_json::Value = serde_json::from_slice(projected.core_json()).unwrap();
    assert_eq!(core["formatVersion"], 4);
    assert!(core.get("$meta").is_none());
    assert_eq!(
        core["distribution"]["Library"]["packageName"],
        "acme/metadata"
    );
    assert_eq!(
        core["distribution"]["Library"]["def"]["modules"]["lifecycle"]["Public"]["values"]["deprecated"]
            ["Public"]["ExpressionBody"]["outputType"],
        "morphir/SDK:basics#bool"
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(projected.document_metadata_json().unwrap()).unwrap();
    let metadata_bytes = projected.document_metadata_json().unwrap();
    assert!(
        provider
            .native
            .windows(metadata_bytes.len())
            .any(|part| part == metadata_bytes)
    );
    let native: serde_json::Value = serde_json::from_slice(&provider.native).unwrap();
    assert_eq!(metadata, native["$meta"]);
    assert_eq!(metadata["@context"], "./contexts/schema.jsonld");
    assert_eq!(metadata["@graph"][0]["@id"], mothers::PREDICATE);
    assert_ne!(
        projected.native_digest(),
        &morphir_package::digest::Digest::of_bytes(projected.core_json())
    );
}

#[tokio::test]
async fn a_signed_naming_release_has_a_closed_consumer_compatible_projection() {
    let provider = mothers::a_signed_naming_provider().await;
    let snapshot = mvp::restore_snapshot(RestoreRequest {
        policy: &provider.policy,
        lock: &provider.lock,
        registry: &provider.directory.path().join("registry"),
        state: &provider.directory.path().join("trust"),
        output: &provider.directory.path().join("out"),
    })
    .await
    .unwrap();
    let library = &snapshot.libraries()[0];
    assert_eq!(
        library.release().package_path().as_str(),
        "example.com/morphir/naming"
    );
    let projection = project_library(library).unwrap();
    let core: serde_json::Value = serde_json::from_slice(projection.core_json()).unwrap();
    assert_eq!(
        core["distribution"]["Library"]["packageName"],
        "morphir/naming"
    );
    let definition = &core["distribution"]["Library"]["def"]["modules"]["naming"]["Public"]["values"]
        ["target-names"]["Public"]["ExpressionBody"];
    assert_eq!(
        definition["outputType"]["Record"]["fields"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
    for field in ["frontend", "backend"] {
        let dictionary = &definition["outputType"]["Record"]["fields"][field]["Reference"];
        assert_eq!(
            dictionary,
            &json!([
                "morphir/SDK:dict#dict",
                "morphir/SDK:string#string",
                "morphir/SDK:string#string"
            ])
        );
    }
    let metadata: serde_json::Value =
        serde_json::from_slice(projection.document_metadata_json().unwrap()).unwrap();
    assert_eq!(
        metadata["@graph"][0]["interpreter"],
        "target-name-language-ids"
    );
    assert_eq!(metadata["@graph"][0]["subject-role"], "ValueDefinition");
}

#[tokio::test]
async fn unsupported_node_metadata_is_refused_instead_of_silently_removed() {
    let provider =
        mothers::a_signed_provider_with(|native| {
            native["distribution"]["Library"]["def"]["modules"]["lifecycle"]["Public"]["values"]
                ["deprecated"]["Public"]["ExpressionBody"]["outputType"] = json!({
                "Reference": {
                    "attributes": {"facts": {(mothers::PREDICATE): true}},
                    "fqname": "morphir/SDK:basics#bool", "args": []
                }
            });
        })
        .await;
    let snapshot = mvp::restore_snapshot(RestoreRequest {
        policy: &provider.policy,
        lock: &provider.lock,
        registry: &provider.directory.path().join("registry"),
        state: &provider.directory.path().join("trust"),
        output: &provider.directory.path().join("out"),
    })
    .await
    .unwrap();
    assert!(matches!(
        project_library(&snapshot.libraries()[0]),
        Err(ProjectionError::NodeLocalMetadata)
    ));
}

#[tokio::test]
async fn nonlexical_parameters_are_refused_instead_of_reordered() {
    let provider =
        mothers::a_signed_provider_with(|native| {
            let mut inputs = serde_json::Map::new();
            for name in ["z", "a"] {
                inputs.insert(name.into(), json!("morphir/SDK:basics#bool"));
            }
            native["distribution"]["Library"]["def"]["modules"]["lifecycle"]["Public"]["values"]
                ["deprecated"]["Public"]["ExpressionBody"]["inputTypes"] =
                serde_json::Value::Object(inputs);
        })
        .await;
    let snapshot = mvp::restore_snapshot(RestoreRequest {
        policy: &provider.policy,
        lock: &provider.lock,
        registry: &provider.directory.path().join("registry"),
        state: &provider.directory.path().join("trust"),
        output: &provider.directory.path().join("out"),
    })
    .await
    .unwrap();
    assert!(matches!(
        project_library(&snapshot.libraries()[0]),
        Err(ProjectionError::ParameterOrder)
    ));
}

#[tokio::test]
async fn frozen_signed_naming_corpus_retains_its_native_revision_and_projection() {
    use std::{fs, path::Path};
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/naming");
    let directory = tempfile::tempdir().unwrap();
    let policy = fs::read(fixture.join("trust-policy.json")).unwrap();
    let lock = fs::read(fixture.join("morphir.lock")).unwrap();
    let state = directory.path().join("trust");
    mvp::initialize(mvp::InitializeRequest {
        policy: &policy,
        root: &fs::read(fixture.join("registry/metadata/1.root.json")).unwrap(),
        state: &state,
    })
    .unwrap();
    let snapshot = mvp::restore_snapshot(RestoreRequest {
        policy: &policy,
        lock: &lock,
        registry: &fixture.join("registry"),
        state: &state,
        output: &directory.path().join("out"),
    })
    .await
    .unwrap();
    let library = &snapshot.libraries()[0];
    assert_eq!(
        library.payload().bytes(),
        fs::read(fixture.join("native.json")).unwrap()
    );
    let projected = project_library(library).unwrap();
    assert_eq!(
        projected.native_digest().to_string(),
        "sha256:870af5f51087dfe6a13fdf10c3b856d879b53d650982612a7aede276060c0720"
    );
    assert_eq!(
        projected.core_json(),
        fs::read(fixture.join("core.json")).unwrap()
    );
    assert_eq!(
        projected.document_metadata_json().unwrap(),
        fs::read(fixture.join("document-metadata.json")).unwrap()
    );
}

#[tokio::test]
async fn an_authenticated_legacy_library_is_not_relabelled_as_native_4_1() {
    use std::{fs, path::Path};
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/local_registry/mvp-fixture");
    let directory = tempfile::tempdir().unwrap();
    let policy = fs::read_to_string(fixture.join("trust-policy.json"))
        .unwrap()
        .replace("previous-authorization", "fresh-metadata")
        .into_bytes();
    let lock = fs::read(fixture.join("morphir.lock")).unwrap();
    let state = directory.path().join("trust");
    mvp::initialize(mvp::InitializeRequest {
        policy: &policy,
        root: &fs::read(fixture.join("registry/metadata/1.root.json")).unwrap(),
        state: &state,
    })
    .unwrap();
    let snapshot = mvp::restore_snapshot(RestoreRequest {
        policy: &policy,
        lock: &lock,
        registry: &fixture.join("registry"),
        state: &state,
        output: &directory.path().join("out"),
    })
    .await
    .unwrap();
    assert!(matches!(
        project_library(&snapshot.libraries()[0]),
        Err(ProjectionError::NativeFormat)
    ));
}

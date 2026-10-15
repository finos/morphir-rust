use ion_rs::{Element, Struct, v1_0::Binary};
use morphir_package::provider_snapshot::service::{MAX_REQUEST_BYTES, serve_request};

#[path = "local_registry/snapshot_mothers.rs"]
#[allow(dead_code)]
mod mothers;
#[path = "local_registry/tuf_mothers.rs"]
#[allow(dead_code)]
mod tuf_mothers;

fn frame(text: &str) -> Vec<u8> {
    Element::read_one(text).unwrap().encode_as(Binary).unwrap()
}
fn correlation() -> Struct {
    Struct::builder()
        .with_field("invocation", "test-operation")
        .with_field("projectScope", "a".repeat(64))
        .build()
}
fn request(provider: &mothers::SignedProvider, release: &str, output: &str) -> Vec<u8> {
    let root = provider.directory.path();
    Element::from(
        Struct::builder()
            .with_field("invocation", "test-operation")
            .with_field("projectScope", "a".repeat(64))
            .with_field("release", release)
            .with_field("assurance", "portable")
            .with_field("registry", root.join("registry").to_str().unwrap())
            .with_field("state", root.join("trust").to_str().unwrap())
            .with_field("output", root.join(output).to_str().unwrap())
            .with_field("policy", ion_rs::Blob::from(provider.policy.clone()))
            .with_field("lock", ion_rs::Blob::from(provider.lock.clone()))
            .build(),
    )
    .with_annotations(["morphir_provider_snapshot_request_v1"])
    .encode_as(Binary)
    .unwrap()
}

#[tokio::test]
async fn capabilities_use_an_explicit_binary_profile_without_restore_inputs() {
    let bytes = Element::from(correlation())
        .with_annotations(["morphir_provider_capabilities_request_v1"])
        .encode_as(Binary)
        .unwrap();
    let result = serve_request(&bytes).await.unwrap();
    assert!(result.starts_with(&[0xe0, 1, 0, 0xea]));
    let value = Element::read_one(result).unwrap();
    assert_eq!(
        value.annotations().iter().next().unwrap().text(),
        Some("morphir_provider_capabilities_response_v1")
    );
    let fields = value.as_struct().unwrap();
    assert_eq!(
        fields.get("invocation").unwrap().as_string(),
        Some("test-operation")
    );
    assert_eq!(
        fields.get("profile").unwrap().as_string(),
        Some("local-library-mvp")
    );
}

#[tokio::test]
async fn signed_naming_snapshot_returns_original_blobs_and_checked_core() {
    let provider = mothers::a_signed_naming_provider().await;
    let bytes = request(
        &provider,
        "example.com/morphir/naming@1.0.0",
        "service-output",
    );
    let result = serve_request(&bytes).await.unwrap();
    let value = Element::read_one(result).unwrap();
    let fields = value.as_struct().unwrap();
    assert_eq!(
        fields.get("release").unwrap().as_string(),
        Some("example.com/morphir/naming@1.0.0")
    );
    assert_eq!(
        fields.get("policySha256").unwrap().as_string(),
        Some(
            morphir_package::digest::Digest::of_bytes(&provider.policy)
                .to_string()
                .as_str()
        )
    );
    let libraries = fields.get("libraries").unwrap().as_list().unwrap();
    let root = libraries.elements().next().unwrap().as_struct().unwrap();
    assert_eq!(
        root.get("native").unwrap().as_blob(),
        Some(provider.native.as_slice())
    );
    let core: serde_json::Value =
        serde_json::from_slice(fields.get("core").unwrap().as_blob().unwrap()).unwrap();
    assert_eq!(core["formatVersion"], 4);
    assert_eq!(
        core["distribution"]["Library"]["packageName"],
        "morphir/naming"
    );
    assert_eq!(
        fields.get("codec").unwrap().as_string(),
        Some("v4.1-document-metadata-to-v4.0/1")
    );
    let contexts = root.get("contexts").unwrap().as_list().unwrap();
    assert_eq!(
        contexts
            .elements()
            .next()
            .unwrap()
            .as_struct()
            .unwrap()
            .get("bytes")
            .unwrap()
            .as_blob(),
        Some(mothers::CONTEXT)
    );
}

#[tokio::test]
async fn each_request_requires_fresh_authentication_and_wrong_release_never_restores() {
    let provider = mothers::a_signed_naming_provider().await;
    assert!(
        serve_request(&request(
            &provider,
            "example.com/morphir/other@1.0.0",
            "wrong-output"
        ))
        .await
        .is_err()
    );
    assert!(!provider.directory.path().join("wrong-output").exists());
    serve_request(&request(
        &provider,
        "example.com/morphir/naming@1.0.0",
        "first-output",
    ))
    .await
    .unwrap();
    std::fs::write(
        provider
            .directory
            .path()
            .join("registry/metadata/timestamp.json"),
        b"{}",
    )
    .unwrap();
    assert!(
        serve_request(&request(
            &provider,
            "example.com/morphir/naming@1.0.0",
            "second-output"
        ))
        .await
        .is_err()
    );
    assert!(!provider.directory.path().join("second-output").exists());
}

#[tokio::test]
async fn framing_refuses_text_duplicates_unknown_fields_annotations_and_extra_values() {
    for text in [
        "morphir_provider_capabilities_request_v1::{invocation:\"x\",projectScope:\"a\",invocation:\"y\"}",
        "morphir_provider_capabilities_request_v1::{invocation:\"x\",projectScope:\"a\",extra:{deep:[[]]}}",
        "wrong::{invocation:\"x\",projectScope:\"a\"}",
        "morphir_provider_capabilities_request_v1::{invocation:tag::\"x\",projectScope:\"a\"}",
    ] {
        assert!(serve_request(&frame(text)).await.is_err());
    }
    assert!(serve_request(b"{}").await.is_err());
    let mut extra = frame(
        "morphir_provider_capabilities_request_v1::{invocation:\"x\",projectScope:\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}",
    );
    extra.extend(frame("null"));
    assert!(serve_request(&extra).await.is_err());
    assert!(
        serve_request(&vec![0; MAX_REQUEST_BYTES + 1])
            .await
            .is_err()
    );
}

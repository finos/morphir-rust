use morphir_package::authoring::AuthoredLibrary;
use serde_json::{Value, json};

fn source() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "formatVersion":4,
        "distribution":{"Library":{"packageName":"example/greeting","dependencies":{},
            "def":{"modules":{"greeting":{"Public":{"types":{},"values":{}}}}}}}
    }))
    .unwrap()
}
fn input() -> Vec<u8> {
    serde_json::to_vec(
        &json!({"packagePath":"example.com/greeting","version":"1.0.0",
        "dependencies":{},"exports":{"greeting":"greeting"}}),
    )
    .unwrap()
}

#[test]
fn creates_verified_deterministic_library_from_exact_ir() {
    let ir = source();
    let library = AuthoredLibrary::create(&input(), &ir).unwrap();
    let manifest: Value = serde_json::from_slice(library.manifest_bytes()).unwrap();
    assert_eq!(manifest["ir"]["packageName"], "example/greeting");
    assert_eq!(
        manifest["content"]["ir.json"],
        morphir_package::digest::Digest::of_bytes(&ir).to_string()
    );
    assert_eq!(library.ir_bytes(), ir);
    assert_eq!(
        AuthoredLibrary::create(&input(), &ir)
            .unwrap()
            .manifest_bytes(),
        library.manifest_bytes()
    );
    assert!(AuthoredLibrary::from_bundle(library.manifest_bytes(), &ir).is_ok());
}

#[test]
fn creates_signed_library_with_inventoried_context_resources() {
    use morphir_package::authoring::LocalSigningKey;
    let mut ir: Value = serde_json::from_slice(&source()).unwrap();
    ir["formatVersion"] = json!("4.1.0");
    let ir = serde_json::to_vec(&ir).unwrap();
    let contexts = vec![(
        "contexts/names.jsonld".to_owned(),
        br#"{"@context":{"alias":"morphir://example/alias"}}"#.to_vec(),
    )];
    let library = AuthoredLibrary::create_with_contexts(&input(), &ir, contexts.clone()).unwrap();
    let manifest: Value = serde_json::from_slice(library.manifest_bytes()).unwrap();
    assert_eq!(manifest["formatVersion"], "0.1.0-draft.2");
    assert_eq!(
        manifest["contextResources"]["contexts/names.jsonld"]["digest"],
        morphir_package::digest::Digest::of_bytes(&contexts[0].1).to_string()
    );
    assert_eq!(library.context_files().count(), 1);
    let signed = library.sign(&LocalSigningKey::from_seed([7; 32])).unwrap();
    assert!(!signed.record_bytes().is_empty());
    assert!(
        AuthoredLibrary::from_bundle_with_contexts(library.manifest_bytes(), &ir, contexts.clone())
            .is_ok()
    );
    for missing_or_changed in [vec![], vec![(contexts[0].0.clone(), b"changed".to_vec())]] {
        assert!(
            AuthoredLibrary::from_bundle_with_contexts(
                library.manifest_bytes(),
                &ir,
                missing_or_changed
            )
            .is_err()
        );
    }
    assert!(AuthoredLibrary::from_bundle(library.manifest_bytes(), &ir).is_err());
    let duplicate = vec![contexts[0].clone(), contexts[0].clone()];
    assert!(AuthoredLibrary::create_with_contexts(&input(), &ir, duplicate).is_err());
    for unsafe_path in [
        "../outside.jsonld",
        "/outside.jsonld",
        "contexts/../../outside.jsonld",
    ] {
        assert!(
            AuthoredLibrary::create_with_contexts(
                &input(),
                &ir,
                vec![(unsafe_path.into(), contexts[0].1.clone())]
            )
            .is_err(),
            "{unsafe_path}"
        );
    }
    let mut extra = contexts;
    extra.push((
        "contexts/extra.jsonld".into(),
        br#"{"@context":{}}"#.to_vec(),
    ));
    assert!(
        AuthoredLibrary::from_bundle_with_contexts(library.manifest_bytes(), &ir, extra).is_err()
    );
}

#[test]
fn rejects_authoring_dependencies_and_unknown_fields() {
    for (key, value) in [
        ("dependencies", json!({"example/other":{}})),
        ("content", json!({})),
        ("version", json!("1.0.0-beta")),
    ] {
        let mut config: Value = serde_json::from_slice(&input()).unwrap();
        config[key] = value;
        assert!(AuthoredLibrary::create(&serde_json::to_vec(&config).unwrap(), &source()).is_err());
    }
}

#[test]
fn refuses_forged_identity_content_and_exports() {
    let library = AuthoredLibrary::create(&input(), &source()).unwrap();
    for pointer in ["/ir/packageName", "/content/ir.json", "/exports/greeting"] {
        let mut manifest: Value = serde_json::from_slice(library.manifest_bytes()).unwrap();
        *manifest.pointer_mut(pointer).unwrap() = json!("wrong");
        assert!(
            AuthoredLibrary::from_bundle(
                &serde_json::to_vec(&manifest).unwrap(),
                library.ir_bytes()
            )
            .is_err()
        );
    }
}

#[test]
fn rejects_duplicate_configuration_keys() {
    assert!(AuthoredLibrary::create(br#"{"packagePath":"example.com/greeting","packagePath":"example.com/other","version":"1.0.0","dependencies":{},"exports":{}}"#, &source()).is_err());
}

#[test]
fn release_signing_verifies_under_independent_publisher_policy() {
    use morphir_package::{
        authoring::LocalSigningKey,
        local_registry::*,
        resolution::{PackagePath, ReleaseId, StableVersion},
    };
    let library = AuthoredLibrary::create(&input(), &source()).unwrap();
    let key = LocalSigningKey::from_seed([7; 32]);
    let signed = library.sign(&key).unwrap();
    let policy = decode_trust_policy(&serde_json::to_vec(&json!({"formatVersion":"0.1.0-draft.3",
        "kind":"LibraryTrustPolicy","repositories":[],"publisherRules":[{"namespace":"example.com",
        "publicKeys":[key.public_key_hex()],"threshold":1}],"continuedUse":"previous-authorization"})).unwrap()).unwrap();
    let subject = ObjectSubject {
        registry: LocalId::parse("local").unwrap(),
        path: RegistryPath::parse("statements/test.json").unwrap(),
    };
    let release = ReleaseId::new(
        PackagePath::parse("example.com/greeting").unwrap(),
        StableVersion::parse("1.0.0").unwrap(),
    );
    let prepared = prepare_publisher_envelope(signed.envelope_bytes(), &subject).unwrap();
    let evidence = verify_publisher_signatures(&prepared, &release, &policy).unwrap();
    let payload: Value = serde_json::from_slice(evidence.payload_bytes()).unwrap();
    assert_eq!(
        payload["manifestDigest"],
        library.metadata().manifest_digest().to_string()
    );
    assert_eq!(
        payload["contentDigest"],
        library.metadata().content_digest().to_string()
    );
    assert!(decode_registry_record(signed.record_bytes(), &Subject::from(&subject)).is_ok());
    let wrong = library.sign(&LocalSigningKey::from_seed([8; 32])).unwrap();
    assert!(
        verify_publisher_signatures(
            &prepare_publisher_envelope(wrong.envelope_bytes(), &subject).unwrap(),
            &release,
            &policy
        )
        .is_err()
    );
}

#[test]
fn repository_signer_uses_tuf_canonicalization_and_is_separate_from_publisher() {
    use morphir_package::{
        authoring::LocalSigningKey,
        local_registry::{
            TufRole,
            tuf::{decode_profile, verify_quorum},
        },
    };
    let key = LocalSigningKey::from_seed([9; 32]);
    let id = key.tuf_key_id().unwrap();
    let root = json!({"_type":"root","spec_version":"1.0.36","version":1,"expires":"2099-01-01T00:00:00Z",
        "consistent_snapshot":true,"keys":{id.clone():key.tuf_public_key()},
        "roles":{"root":{"keyids":[id.clone()],"threshold":1},"targets":{"keyids":[id.clone()],"threshold":1},
        "snapshot":{"keyids":[id.clone()],"threshold":1},"timestamp":{"keyids":[id],"threshold":1}}});
    let signed = key.sign_tuf(&root).unwrap();
    let profile = decode_profile(&signed, TufRole::Root).unwrap();
    assert!(verify_quorum(&profile, &profile).is_ok());
    let targets = json!({"_type":"targets","spec_version":"1.0.36","version":1,"expires":"2099-01-01T00:00:00Z","targets":{}});
    let signed = key.sign_tuf(&targets).unwrap();
    assert!(
        verify_quorum(
            &profile,
            &decode_profile(&signed, TufRole::Targets).unwrap()
        )
        .is_ok()
    );
    let wrong = LocalSigningKey::from_seed([10; 32])
        .sign_tuf(&targets)
        .unwrap();
    assert!(verify_quorum(&profile, &decode_profile(&wrong, TufRole::Targets).unwrap()).is_err());
}

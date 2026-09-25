use morphir_package::{
    digest::Digest,
    library::{LibraryInput, VerifiedLibrarySet},
    metadata::NormalizedMetadata,
    schema::PackageSchemas,
};
use serde_json::{Value, json};

struct LibraryMother {
    manifest: Value,
    ir: Value,
}

fn a_provider() -> LibraryMother {
    LibraryMother {
        manifest: json!({"formatVersion":"0.1.0-draft.1","kind":"Library","packagePath":"example.com/provider","version":"1.0.0","ir":{"formatVersion":"4","packageName":"example/provider","payload":{"path":"ir.json","mediaType":"application/json","profile":"classic"}},"dependencies":{},"exports":{"api":"api"},"content":{}}),
        ir: json!({"formatVersion":4,"distribution":{"Library":{"packageName":"example/provider","dependencies":{},"def":{"modules":{"api":{"Public":{"types":{},"values":{}}}}}}}}),
    }
}

fn a_consumer() -> LibraryMother {
    let mut library = a_provider();
    library.manifest["packagePath"] = json!("example.com/consumer");
    library.manifest["ir"]["packageName"] = json!("example/consumer");
    library.manifest["dependencies"] = json!({"example/provider":{"packagePath":"example.com/provider","versionRange":{"minimumInclusive":"1.0.0","maximumExclusive":"2.0.0"}}});
    library.ir["distribution"]["Library"]["packageName"] = json!("example/consumer");
    library.ir["distribution"]["Library"]["dependencies"] =
        json!({"example/provider":{"modules":{}}});
    library
}

fn a_closed_set(mut libraries: Vec<LibraryMother>) -> (Value, Vec<LibraryInput>) {
    let mut nodes = serde_json::Map::new();
    let inputs = libraries.iter_mut().enumerate().map(|(index, library)| {
        let bytes = library.ir.to_string().into_bytes();
        library.manifest["content"] = json!({"ir.json":Digest::of_bytes(&bytes).to_string()});
        let text = library.manifest.to_string();
        let metadata = NormalizedMetadata::parse(&text).unwrap();
        let bindings = if library.manifest["dependencies"].as_object().unwrap().is_empty() { json!({}) } else { json!({"example/provider":"n0"}) };
        nodes.insert(format!("n{index}"), json!({"release":{"packagePath":library.manifest["packagePath"],"version":library.manifest["version"]},"irPackageName":library.manifest["ir"]["packageName"],"manifestDigest":metadata.manifest_digest().to_string(),"contentDigest":metadata.content_digest().to_string(),"bindings":bindings}));
        LibraryInput::new(text, vec![("ir.json".into(), bytes)])
    }).collect();
    (
        json!({"formatVersion":"0.1.0-draft.1","kind":"LibraryLockCore","root":"n0","nodes":nodes}),
        inputs,
    )
}

fn verify(lock: &Value, inputs: &[LibraryInput]) -> bool {
    // These tests isolate semantic integrity from the externally supplied schemas.
    let schemas = PackageSchemas::compile(&json!({}), &json!({})).unwrap();
    VerifiedLibrarySet::verify(&schemas, &lock.to_string(), inputs).is_ok()
}

#[test]
fn verifies_closed_set_and_rejects_dangling_or_inconsistent_nodes() {
    let (lock, inputs) = a_closed_set(vec![a_provider(), a_consumer()]);
    assert!(verify(&lock, &inputs));
    for (pointer, replacement) in [
        ("/root", json!("n9")),
        ("/nodes/n1/bindings/example~1provider", json!("n9")),
        ("/nodes/n1/bindings", json!({})),
        ("/nodes/n0/irPackageName", json!("example/other")),
        (
            "/nodes/n0/manifestDigest",
            json!("sha256:0000000000000000000000000000000000000000000000000000000000000000"),
        ),
        (
            "/nodes/n0/contentDigest",
            json!("sha256:0000000000000000000000000000000000000000000000000000000000000000"),
        ),
        ("/nodes/n0/release/version", json!("2.0.0")),
    ] {
        let mut changed = lock.clone();
        *changed.pointer_mut(pointer).unwrap() = replacement;
        assert!(!verify(&changed, &inputs), "{pointer}");
    }
    assert!(!verify(&lock, &inputs[..1]));
    assert!(!verify(&lock, &[inputs[0].clone(), inputs[0].clone()]));
}

#[test]
fn rejects_payload_errors_even_with_fresh_file_and_manifest_digests() {
    for (pointer, replacement) in [
        ("/formatVersion", json!(3)),
        ("/distribution/Library/packageName", json!("example/other")),
        (
            "/distribution/Library/dependencies",
            json!({"example/other":{"modules":{}}}),
        ),
        (
            "/distribution/Library/def/modules/api",
            json!({"Private":{"types":{},"values":{}}}),
        ),
        ("/distribution/Library/def/modules", json!({})),
        (
            "/distribution",
            json!({"Specs":{"packageName":"example/provider","dependencies":{},"spec":{"modules":{}}}}),
        ),
        (
            "/distribution/Library/def/modules/api/Public/values",
            json!({"invalid":42}),
        ),
    ] {
        let mut provider = a_provider();
        *provider.ir.pointer_mut(pointer).unwrap() = replacement;
        let (lock, inputs) = a_closed_set(vec![provider]);
        assert!(!verify(&lock, &inputs), "{pointer}");
    }
    let mut provider = a_provider();
    let model = provider.ir["distribution"]["Library"]
        .as_object_mut()
        .unwrap();
    let definition = model.remove("def").unwrap();
    model.insert("definition".into(), definition);
    let (lock, inputs) = a_closed_set(vec![provider]);
    assert!(
        !verify(&lock, &inputs),
        "legacy spelling warnings must reject"
    );
}

#[test]
fn accepts_current_codec_version_four_string_spelling() {
    let mut provider = a_provider();
    provider.ir["formatVersion"] = json!("4.0.0");
    let (lock, inputs) = a_closed_set(vec![provider]);
    assert!(verify(&lock, &inputs));
}

#[test]
fn versioned_context_inventory_binds_media_and_raw_digest() {
    let mut provider = a_provider();
    provider.ir["formatVersion"] = json!("4.1.0");
    provider.manifest["formatVersion"] = json!("0.1.0-draft.2");
    let ir = provider.ir.to_string().into_bytes();
    let context = br#"{"@context":{"label":"morphir://example/label"}}"#.to_vec();
    provider.manifest["content"] = json!({
        "ir.json": Digest::of_bytes(&ir).to_string(),
        "contexts/names.jsonld": Digest::of_bytes(&context).to_string()
    });
    provider.manifest["contextResources"] = json!({
        "contexts/names.jsonld": {
            "mediaType": "application/ld+json",
            "digest": Digest::of_bytes(&context).to_string()
        }
    });
    let metadata = NormalizedMetadata::parse(&provider.manifest.to_string()).unwrap();
    let lock = json!({"formatVersion":"0.1.0-draft.1","kind":"LibraryLockCore","root":"n0","nodes":{"n0":{
        "release":{"packagePath":"example.com/provider","version":"1.0.0"},
        "irPackageName":"example/provider",
        "manifestDigest":metadata.manifest_digest().to_string(),
        "contentDigest":metadata.content_digest().to_string(),"bindings":{}
    }}});
    let input = |manifest: &Value| {
        LibraryInput::new(
            manifest.to_string(),
            vec![
                ("ir.json".into(), ir.clone()),
                ("contexts/names.jsonld".into(), context.clone()),
            ],
        )
    };
    assert!(verify(&lock, &[input(&provider.manifest)]));
    for (pointer, replacement) in [
        (
            "/contextResources/contexts~1names.jsonld/digest",
            json!(Digest::of_bytes(b"changed").to_string()),
        ),
        (
            "/contextResources/contexts~1names.jsonld/mediaType",
            json!("text/plain"),
        ),
    ] {
        let mut changed = provider.manifest.clone();
        *changed.pointer_mut(pointer).unwrap() = replacement;
        let normalized = NormalizedMetadata::parse(&changed.to_string()).unwrap();
        let mut relocked = lock.clone();
        relocked["nodes"]["n0"]["manifestDigest"] = json!(normalized.manifest_digest().to_string());
        relocked["nodes"]["n0"]["contentDigest"] = json!(normalized.content_digest().to_string());
        assert!(!verify(&relocked, &[input(&changed)]), "{pointer}");
    }
    let mut legacy = provider.manifest.clone();
    legacy["formatVersion"] = json!("0.1.0-draft.1");
    legacy.as_object_mut().unwrap().remove("contextResources");
    let normalized = NormalizedMetadata::parse(&legacy.to_string()).unwrap();
    let mut relocked = lock;
    relocked["nodes"]["n0"]["manifestDigest"] = json!(normalized.manifest_digest().to_string());
    relocked["nodes"]["n0"]["contentDigest"] = json!(normalized.content_digest().to_string());
    assert!(
        !verify(&relocked, &[input(&legacy)]),
        "draft.1 must reject context extras"
    );
}

#[test]
fn accepts_ir_payload_deeper_than_serde_default_limit() {
    let mut provider = a_provider();
    let mut tpe = json!({"Unit":{}});
    for _ in 0..150 {
        tpe = json!({"Tuple":{"elements":[tpe]}});
    }
    provider.ir["distribution"]["Library"]["def"]["modules"]["api"]["Public"]["types"] =
        json!({"nested":{"Public":{"TypeAliasDefinition":{"typeParams":[],"typeExp":tpe}}}});
    let (lock, inputs) = a_closed_set(vec![provider]);
    assert!(verify(&lock, &inputs));
}

#[test]
fn version_ranges_compare_arbitrary_precision_components() {
    for (version, minimum, maximum, accepted) in [
        (
            "999999999999999999999999999999.0.0",
            "999999999999999999999999999999.0.0",
            "1000000000000000000000000000000.0.0",
            true,
        ),
        (
            "1.0.9007199254740992",
            "1.0.9007199254740993",
            "2.0.0",
            false,
        ),
        (
            "1.0.9007199254740993",
            "1.0.9007199254740992",
            "1.0.9007199254740993",
            false,
        ),
        ("1.0.0", "1.0.0", "1.0.0", false),
        ("1.0.0", "2.0.0", "1.0.0", false),
    ] {
        let mut provider = a_provider();
        provider.manifest["version"] = json!(version);
        let mut consumer = a_consumer();
        consumer.manifest["dependencies"]["example/provider"]["versionRange"] =
            json!({"minimumInclusive":minimum,"maximumExclusive":maximum});
        let (lock, inputs) = a_closed_set(vec![provider, consumer]);
        assert_eq!(
            verify(&lock, &inputs),
            accepted,
            "{version} in [{minimum}, {maximum})"
        );
    }
}

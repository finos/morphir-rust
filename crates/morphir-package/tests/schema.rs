use morphir_package::schema::{Artifact, PackageSchemas};
use serde_json::json;

#[test]
fn local_references_resolve_without_retrieval() {
    let manifest = json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.invalid/manifest.json","$defs":{"version":{"type":"integer"}},"type":"object"});
    let lock =
        json!({"$id":"https://example.invalid/lock.json","$ref":"manifest.json#/$defs/version"});
    let schemas = PackageSchemas::compile(&manifest, &lock).unwrap();
    assert!(schemas.validate(Artifact::Manifest, "{}"));
    assert!(schemas.validate(Artifact::Lock, "123"));
    assert!(!schemas.validate(Artifact::Lock, r#""123""#));
    assert!(!schemas.validate(Artifact::Manifest, r#"{"a":1,"\u0061":2}"#));
    assert!(!schemas.validate(Artifact::Manifest, "\u{feff}{}"));
}

#[test]
fn invalid_schemas_and_unresolved_refs_are_errors() {
    for invalid in [
        json!({"type":17}),
        json!({"$ref":"file:///etc/passwd"}),
        json!({"$ref":"https://example.invalid/missing.json"}),
        json!({"$ref":"#/$defs/missing"}),
        json!({"$defs":{"unused":{"type":17}}}),
    ] {
        assert!(
            PackageSchemas::compile(&invalid, &json!({})).is_err(),
            "{invalid}"
        );
        assert!(
            PackageSchemas::compile(&json!({}), &invalid).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn schema_validation_does_not_impose_metadata_normalization() {
    let schema = json!({});
    let schemas = PackageSchemas::compile(&schema, &schema).unwrap();
    for input in ["true", "null", "42", r#""\n""#, r#""é""#] {
        assert!(schemas.validate(Artifact::Manifest, input), "{input}");
    }
}

#[test]
fn schema_validation_retains_current_json_nesting_limit() {
    let schemas = PackageSchemas::compile(&json!({}), &json!({})).unwrap();
    for depth in [129, 999, 1000, 1001] {
        let input = format!("{}null{}", "[".repeat(depth), "]".repeat(depth));
        assert_eq!(
            schemas.validate(Artifact::Manifest, &input),
            depth <= 1000,
            "depth {depth}"
        );
    }
}

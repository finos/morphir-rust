use morphir_package::{digest::Digest, metadata::NormalizedMetadata, strict_json};

#[test]
fn root_depth_zero_allows_64_edges_and_rejects_65() {
    for depth in [0, 1, 63, 64, 65] {
        let text = format!("{}\"x\"{}", "[".repeat(depth), "]".repeat(depth));
        assert_eq!(NormalizedMetadata::parse(&text).is_ok(), depth <= 64);
    }
}

#[test]
fn private_number_token_keys_are_ordinary_objects() {
    let input = r#"{"$serde_json::private::Number":"123"}"#;
    assert_eq!(NormalizedMetadata::parse(input).unwrap().as_str(), input);
    assert!(NormalizedMetadata::parse("123456789012345678901234567890").is_err());
    assert!(
        strict_json::parse(r#"{"$serde_json::private::Number":"123","x":{"a":1,"a":2}}"#).is_err()
    );
}

#[test]
fn exact_byte_hashes_include_whitespace_and_newlines() {
    assert_eq!(
        Digest::of_bytes(b"abc").to_string(),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_ne!(Digest::of_bytes(b"abc"), Digest::of_bytes(b"abc\n"));
    assert_ne!(Digest::of_bytes(b"abc\n"), Digest::of_bytes(b"abc\r\n"));
    let a = NormalizedMetadata::parse(r#"{"x":"/\\\""}"#).unwrap();
    let b = NormalizedMetadata::parse(r#"{ "x": "/\\\"" }"#).unwrap();
    assert_eq!(a.manifest_digest(), b.manifest_digest());
    assert_ne!(a.manifest_digest(), a.content_digest());
}

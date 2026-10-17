//! JSON admission keeps exact bytes while accepting IR reader defaults.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use morphir_extension_sdk::model::*;
use sha2::{Digest, Sha256};

fn a_tuple() -> ModelTransportTuple {
    ModelTransportTuple {
        operation: ModelOperation::GenerateModel,
        profile: "morphir-ir".into(),
        codec: ModelCodec::MorphirJson,
        envelope_version: MODEL_ENVELOPE_VERSION.into(),
        ir_version: "4.0.0".into(),
    }
}
fn bounds() -> ModelTransportLimits {
    ModelTransportLimits {
        max_encoded_bytes: 65536,
        max_decoded_bytes: 49152,
        max_depth: 256,
        max_nodes: 4096,
    }
}
fn model(bytes: &[u8]) -> MorphirModel {
    MorphirModel {
        envelope_version: MODEL_ENVELOPE_VERSION.into(),
        model_id: "urn:fixture:model".into(),
        revision: "revision-1".into(),
        ir_version: "4.0.0".into(),
        profile: "morphir-ir".into(),
        payload: ModelPayload {
            codec: ModelCodec::MorphirJson,
            layout: ModelLayout::SingleDocument,
            decoded_byte_length: bytes.len() as u32,
            sha256: Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            content: STANDARD.encode(bytes),
        },
        evidence: Vec::new(),
        critical: Vec::new(),
    }
}

#[test]
fn default_empty_v4_distributions_keep_their_accepted_omitted_spelling() {
    for distribution in [
        r#"{"Library":{"packageName":"example/empty"}}"#,
        r#"{"Library":{"packageName":"example/empty","def":{}}}"#,
        r#"{"Specs":{"packageName":"example/empty"}}"#,
        r#"{"Specs":{"packageName":"example/empty","spec":{}}}"#,
        r#"{"Application":{"packageName":"example/empty","entryPoints":{}}}"#,
    ] {
        let bytes = format!(r#"{{"formatVersion":4,"distribution":{distribution}}}"#).into_bytes();
        assert_eq!(
            retain_model(&model(&bytes), &a_tuple(), &bounds())
                .unwrap()
                .bytes(),
            bytes
        );
    }
}

#[test]
fn optional_defaulted_members_still_require_their_declared_types() {
    for distribution in [
        r#"{"Library":{"packageName":"example/empty","dependencies":null}}"#,
        r#"{"Library":{"packageName":"example/empty","def":[]}}"#,
        r#"{"Library":{"packageName":"example/empty","def":{"modules":[]}}}"#,
        r#"{"Specs":{"packageName":"example/empty","spec":null}}"#,
        r#"{"Specs":{"packageName":"example/empty","spec":{"modules":null}}}"#,
        r#"{"Application":{"packageName":"example/empty"}}"#,
    ] {
        let bytes = format!(r#"{{"formatVersion":4,"distribution":{distribution}}}"#).into_bytes();
        assert!(
            retain_model(&model(&bytes), &a_tuple(), &bounds()).is_err(),
            "{distribution}"
        );
    }
}

#[test]
fn retained_json_does_not_rewrite_arbitrary_precision_numeric_lexemes() {
    let bytes = br#"{"formatVersion":4,"distribution":{"Library":{"packageName":"example/empty"}},"future":{"integer":1234567890123456789012345678901234567890,"decimal":0.1234567890123456789012345678900000}}"#;
    assert_eq!(
        retain_model(&model(bytes), &a_tuple(), &bounds())
            .unwrap()
            .bytes(),
        bytes
    );
}

#[test]
fn classic_distribution_shape_and_specs_version_match_the_ion_boundary() {
    for (version, distribution) in [
        ("3.0.0", r#"["Library",[["example"]],[],{"modules":[]}]"#),
        ("3.1.0", r#"["Specs",[["example"]],[],{"modules":[]}]"#),
    ] {
        let bytes = format!(r#"{{"formatVersion":"{version}","distribution":{distribution}}}"#)
            .into_bytes();
        let mut original = model(&bytes);
        original.ir_version = version.into();
        let mut tuple = a_tuple();
        tuple.ir_version = version.into();
        assert_eq!(
            retain_model(&original, &tuple, &bounds()).unwrap().bytes(),
            bytes
        );
    }
    let bytes = br#"{"formatVersion":3,"distribution":["Specs",[],[],{"modules":[]}]}"#;
    let mut original = model(bytes);
    original.ir_version = "3.0.0".into();
    let mut tuple = a_tuple();
    tuple.ir_version = "3.0.0".into();
    assert!(retain_model(&original, &tuple, &bounds()).is_err());
}

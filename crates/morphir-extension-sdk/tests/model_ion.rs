//! Ion admission preserves rich content while bounding structural inspection.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ion_rs::{Element, v1_0::Binary};
use morphir_extension_sdk::model::*;
use sha2::{Digest, Sha256};

fn a_tuple() -> ModelTransportTuple {
    ModelTransportTuple {
        operation: ModelOperation::GenerateModel,
        profile: "morphir-model".into(),
        codec: ModelCodec::IonBinary,
        envelope_version: MODEL_ENVELOPE_VERSION.into(),
        ir_version: "4.0.0".into(),
    }
}

fn limits() -> ModelTransportLimits {
    ModelTransportLimits {
        max_encoded_bytes: 65536,
        max_decoded_bytes: 49152,
        max_depth: 64,
        max_nodes: 4096,
    }
}

fn model(bytes: &[u8]) -> MorphirModel {
    MorphirModel {
        envelope_version: MODEL_ENVELOPE_VERSION.into(),
        model_id: "urn:fixture:model".into(),
        revision: "revision-1".into(),
        ir_version: "4.0.0".into(),
        profile: "morphir-model".into(),
        payload: ModelPayload {
            codec: ModelCodec::IonBinary,
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

fn encode(text: &str) -> Vec<u8> {
    Element::read_one(text).unwrap().encode_as(Binary).unwrap()
}

#[test]
fn genuine_ion_retains_ordered_annotations_exact_numbers_and_unknown_facts() {
    let bytes = include_bytes!("fixtures/model_transport/fixtures/rich-model.ionb");
    let original = model(bytes);
    let retained = retain_model(&original, &a_tuple(), &limits()).unwrap();
    assert_eq!(retained.bytes(), bytes);
    assert_eq!(retained.envelope(), &original);
}

#[test]
fn invalid_ion_and_duplicate_reserved_fields_cannot_be_retained() {
    let genuine = include_bytes!("fixtures/model_transport/fixtures/rich-model.ionb");
    for bytes in [
        b"not binary Ion".to_vec(),
        [genuine.as_slice(), genuine.as_slice()].concat(),
        encode(
            "morphir_model::{ir:{formatVersion:4,distribution:{Library:{packageName:\"x\",dependencies:{},def:{modules:{}}}}},ir:{},enrichments:[],provenance:[]}",
        ),
        encode(
            "morphir_model::{ir:{formatVersion:4,distribution:{Library:{packageName:\"x\",dependencies:{},def:{modules:{}}}}},enrichments:[],provenance:[],provenance:[]}",
        ),
    ] {
        assert!(retain_model(&model(&bytes), &a_tuple(), &limits()).is_err());
    }
}

#[test]
fn lower_tree_limits_refuse_genuine_rich_payloads() {
    let bytes = include_bytes!("fixtures/model_transport/fixtures/rich-model.ionb");
    let mut bounds = limits();
    bounds.max_depth = 2;
    assert!(retain_model(&model(bytes), &a_tuple(), &bounds).is_err());
    bounds = limits();
    bounds.max_nodes = 2;
    assert!(retain_model(&model(bytes), &a_tuple(), &bounds).is_err());
}

#[test]
fn shared_imports_cannot_allocate_symbols_from_untrusted_max_id() {
    let table = Element::read_one(
        "{imports:[{name:\"unavailable\",version:1,max_id:9223372036854775807}]}",
    )
    .unwrap()
    .with_annotations(["$ion_symbol_table"])
    .encode_as(Binary)
    .unwrap();
    let genuine = include_bytes!("fixtures/model_transport/fixtures/rich-model.ionb");
    let bytes = [table.as_slice(), genuine.as_slice()].concat();
    assert!(retain_model(&model(&bytes), &a_tuple(), &limits()).is_err());
}

#[test]
fn default_empty_v4_members_may_be_omitted_without_changing_the_ir_contract() {
    for distribution in [
        "{Library:{packageName:\"example/empty\"}}",
        "{Library:{packageName:\"example/empty\",def:{}}}",
        "{Specs:{packageName:\"example/empty\"}}",
        "{Specs:{packageName:\"example/empty\",spec:{}}}",
        "{Application:{packageName:\"example/empty\",entryPoints:{}}}",
        "{Application:{packageName:\"example/empty\",def:{},entryPoints:{}}}",
    ] {
        let bytes = encode(&format!(
            "morphir_model::{{ir:{{formatVersion:4,distribution:{distribution}}},enrichments:[],provenance:[]}}"
        ));
        let original = model(&bytes);
        let retained = retain_model(&original, &a_tuple(), &limits()).unwrap();
        assert_eq!(retained.bytes(), bytes);
    }
}

#[test]
fn default_empty_members_still_refuse_wrong_types_when_present() {
    for distribution in [
        "{Library:{packageName:\"example/empty\",dependencies:null}}",
        "{Library:{packageName:\"example/empty\",def:[]}}",
        "{Library:{packageName:\"example/empty\",def:{modules:null}}}",
        "{Specs:{packageName:\"example/empty\",spec:[]}}",
        "{Specs:{packageName:\"example/empty\",spec:{modules:[]}}}",
        "{Application:{packageName:\"example/empty\",entryPoints:{},def:null}}",
        "{Application:{packageName:\"example/empty\"}}",
        "{Application:{packageName:\"example/empty\",entryPoints:[]}}",
    ] {
        let bytes = encode(&format!(
            "morphir_model::{{ir:{{formatVersion:4,distribution:{distribution}}},enrichments:[],provenance:[]}}"
        ));
        assert!(
            retain_model(&model(&bytes), &a_tuple(), &limits()).is_err(),
            "{distribution}"
        );
    }
}

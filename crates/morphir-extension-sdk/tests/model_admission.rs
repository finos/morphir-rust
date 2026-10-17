use morphir_extension_sdk::model::*;
use serde_json::Value;

fn corpus() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/model_transport/model-transport.cases.json"
    ))
    .unwrap()
}

fn a_rich_model() -> (MorphirModel, ModelTransportTuple, ModelTransportLimits) {
    let c = corpus();
    let case = &c["cases"][0];
    (
        serde_json::from_value(case["request"]["params"]["model"].clone()).unwrap(),
        serde_json::from_value(case["request"]["params"]["input"].clone()).unwrap(),
        serde_json::from_value(case["hostOffer"]["modelTransport"]["limits"].clone()).unwrap(),
    )
}

#[test]
fn retention_keeps_exact_annotated_binary_bytes() {
    let (model, tuple, limits) = a_rich_model();
    let retained = retain_model(&model, &tuple, &limits).unwrap();
    assert_eq!(
        retained.bytes(),
        include_bytes!("fixtures/model_transport/fixtures/rich-model.ionb")
    );
    assert_eq!(retained.envelope(), &model);
}

#[test]
fn integrity_limits_and_critical_children_refuse_admission() {
    let (model, tuple, limits) = a_rich_model();
    let mut wrong = model.clone();
    wrong.payload.sha256 = "0".repeat(64);
    assert!(retain_model(&wrong, &tuple, &limits).is_err());
    let mut wrong = model.clone();
    wrong.payload.decoded_byte_length += 1;
    assert!(retain_model(&wrong, &tuple, &limits).is_err());
    let mut wrong = model.clone();
    wrong.critical.push("payload.futureMember".into());
    assert!(retain_model(&wrong, &tuple, &limits).is_err());
    let small = ModelTransportLimits {
        max_decoded_bytes: 1,
        ..limits.clone()
    };
    assert!(retain_model(&model, &tuple, &small).is_err());
}

#[test]
fn aggregate_evidence_cannot_bypass_decoded_budget() {
    let c = corpus();
    let case = c["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "model-0035")
        .unwrap();
    let model = serde_json::from_value(case["request"]["params"]["model"].clone()).unwrap();
    let tuple = serde_json::from_value(case["request"]["params"]["input"].clone()).unwrap();
    let limits =
        serde_json::from_value(case["hostOffer"]["modelTransport"]["limits"].clone()).unwrap();
    assert!(retain_model(&model, &tuple, &limits).is_err());
}

#[test]
fn critical_refusal_identifies_the_exact_bounded_member_path() {
    let (mut model, tuple, limits) = a_rich_model();
    model.critical = vec!["payload.futureMember".into()];
    let error = retain_model(&model, &tuple, &limits).unwrap_err();
    assert_eq!(error.code(), "unknown_critical_path");
    assert_eq!(error.path(), Some("payload.futureMember"));
}

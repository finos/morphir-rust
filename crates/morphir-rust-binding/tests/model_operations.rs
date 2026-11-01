//! MEP `0.2.0-draft.1` negotiation and model operations through native JSON-RPC dispatch.

#[path = "support/model.rs"]
mod model;

use model::*;
use morphir_extension_sdk::{
    model::{
        CompileModelResult, MODEL_ENVELOPE_VERSION, MODEL_MEP_VERSION, ModelCodec,
        ModelGuaranteeRequirement, ModelOperation, ModelTransportSupport, MorphirModel,
    },
    prelude::*,
    protocol::{DescribeParams, methods},
};
use morphir_rust_binding::RustExtension;
use serde_json::{Value, json};

fn compiled_model(session: &Session) -> MorphirModel {
    let response = session.call(methods::COMPILE_MODEL, a_compile_model_request());
    assert!(response.error.is_none(), "{:?}", response.error);
    match serde_json::from_value(response.result.unwrap()).unwrap() {
        CompileModelResult::Compiled { model, .. } => *model,
        CompileModelResult::Failed { diagnostics } => panic!("compile failed: {diagnostics:?}"),
    }
}

fn agreed(session: &Session) -> ModelTransportSupport {
    serde_json::from_value(session.result()["capabilities"]["modelTransport"].clone()).unwrap()
}

// ---- negotiation ------------------------------------------------------------------------------

#[test]
fn describe_claims_both_protocol_lines_and_exactly_the_json_tuples() {
    let session = Session::start(&[], None);
    let response = session.call(
        methods::DESCRIBE,
        DescribeParams {
            protocol_versions: vec!["0.1".into()],
        },
    );
    let claims = response.result.expect("describe result");
    let mut versions: Vec<String> =
        serde_json::from_value(claims["protocolVersions"].clone()).unwrap();
    versions.sort();
    assert_eq!(versions, ["0.1", MODEL_MEP_VERSION]);
    let support: ModelTransportSupport =
        serde_json::from_value(claims["capabilities"]["modelTransport"].clone()).unwrap();
    assert_eq!(
        support.tuples,
        [
            a_json_tuple(ModelOperation::CompileModel),
            a_json_tuple(ModelOperation::GenerateModel)
        ]
    );
    assert!(support.guarantees.is_empty());
}

#[test]
fn a_host_that_offers_only_mep_0_1_gets_0_1_without_model_transport() {
    let session = Session::on_0_1();
    assert_eq!(session.result()["protocolVersion"], "0.1");
    assert!(
        session.result()["capabilities"]
            .get("modelTransport")
            .is_none(),
        "{:#}",
        session.result()
    );
}

#[test]
fn a_model_host_gets_the_agreed_json_tuples_in_host_order_and_its_smaller_limits() {
    let session = Session::on_model_line();
    assert_eq!(session.result()["protocolVersion"], MODEL_MEP_VERSION);
    let support = agreed(&session);
    assert_eq!(
        support.tuples,
        [
            a_json_tuple(ModelOperation::CompileModel),
            a_json_tuple(ModelOperation::GenerateModel)
        ]
    );
    assert_eq!(support.limits, moonbit_host_limits());
}

#[test]
fn the_agreement_follows_the_host_order_and_takes_each_smaller_limit() {
    // The host lists generate before compile, and offers more nodes than the guest allows.
    let mut offer = a_moonbit_like_offer();
    offer.tuples.reverse();
    offer.limits.max_nodes = u32::try_from(i32::MAX).unwrap();
    let session = Session::start(&[MODEL_MEP_VERSION, "0.1"], Some(offer));
    let support = agreed(&session);
    assert_eq!(
        support.tuples,
        [
            a_json_tuple(ModelOperation::GenerateModel),
            a_json_tuple(ModelOperation::CompileModel)
        ]
    );
    assert_eq!(support.limits.max_nodes, 16 * 1024 * 1024);
    assert_eq!(
        support.limits.max_decoded_bytes,
        moonbit_host_limits().max_decoded_bytes
    );
}

#[test]
fn a_draft_host_without_a_model_transport_offer_gets_0_1() {
    let session = Session::start(&[MODEL_MEP_VERSION, "0.1"], None);
    assert!(session.initialized.error.is_none());
    assert_eq!(session.result()["protocolVersion"], "0.1");
}

#[test]
fn an_ion_only_offer_agrees_on_no_tuple_and_no_model_operation() {
    let session = Session::start(&[MODEL_MEP_VERSION, "0.1"], Some(an_ion_only_offer()));
    assert!(session.initialized.error.is_none());
    assert!(agreed(&session).tuples.is_empty());
    let response = session.call(methods::COMPILE_MODEL, a_compile_model_request());
    assert!(response.result.is_none() && response.error.is_some());
}

// ---- compileModel and generateModel -----------------------------------------------------------

#[test]
fn compile_model_returns_the_golden_ir_v4_envelope() {
    let session = Session::on_model_line();
    let response = session.call(methods::COMPILE_MODEL, a_compile_model_request());
    let result = response.result.expect("compileModel result");
    assert_eq!(result["success"], true);
    assert_eq!(result["modules"], json!(["Models"]));
    let model: MorphirModel = serde_json::from_value(result["model"].clone()).unwrap();
    assert_eq!(model.envelope_version, MODEL_ENVELOPE_VERSION);
    assert_eq!(model.profile, "morphir-ir");
    assert_eq!(model.ir_version, "4.0.0");
    assert_eq!(model.model_id, "acme/example");
    assert_eq!(model.revision, format!("sha256:{}", model.payload.sha256));
    assert!(model.evidence.is_empty() && model.critical.is_empty());
    let payload = payload_json(&model, ModelOperation::CompileModel);
    assert_golden(
        "compiled-ir-v4.json",
        &format!("{}\n", serde_json::to_string_pretty(&payload).unwrap()),
    );
}

#[test]
fn compile_model_payload_is_the_legacy_ir_v4_result() {
    let session = Session::on_model_line();
    let payload = payload_json(&compiled_model(&session), ModelOperation::CompileModel);
    let legacy = RustExtension.compile(a_legacy_compile_request()).unwrap();
    assert!(legacy.success, "{:?}", legacy.diagnostics);
    assert_eq!(legacy.ir_version.as_deref(), Some("4"));
    assert_eq!(payload, legacy.ir.unwrap());
}

#[test]
fn generate_model_from_the_compiled_envelope_returns_the_golden_rust() {
    let session = Session::on_model_line();
    let model = compiled_model(&session);
    let response = session.call(methods::GENERATE_MODEL, a_generate_model_request(model));
    assert!(response.error.is_none(), "{:?}", response.error);
    let generated: GenerateResult = serde_json::from_value(response.result.unwrap()).unwrap();
    assert!(generated.success, "{:?}", generated.diagnostics);
    assert_eq!(generated.artifacts.len(), 1);
    assert_eq!(generated.artifacts[0].path, "lib.rs");
    assert_golden("generated-lib.rs", &generated.artifacts[0].content);
}

#[test]
fn generate_model_returns_the_legacy_generate_result() {
    let session = Session::on_model_line();
    let model = compiled_model(&session);
    let ir = payload_json(&model, ModelOperation::CompileModel);
    let response = session.call(methods::GENERATE_MODEL, a_generate_model_request(model));
    let rich: Value = response.result.unwrap();
    let legacy = RustExtension
        .generate(GenerateRequest {
            ir,
            target: "rust".into(),
            options: Default::default(),
        })
        .unwrap();
    assert_eq!(rich, serde_json::to_value(legacy).unwrap());
}

#[test]
fn compile_model_failure_has_diagnostics_and_no_model() {
    let session = Session::on_model_line();
    let request = a_compile_model_request_for("models.rs", "pub struct Broken { pub field: &str }");
    let response = session.call(methods::COMPILE_MODEL, request);
    let result = response.result.expect("a failed compilation is a result");
    assert_eq!(result["success"], false);
    assert!(result.get("model").is_none(), "{result:#}");
    assert!(!result["diagnostics"].as_array().unwrap().is_empty());
}

#[test]
fn compile_model_takes_the_module_from_a_relative_directory_uri() {
    let session = Session::on_model_line();
    let request = a_compile_model_request_for("src/domain/models.rs", "pub type Id = String;");
    let response = session.call(methods::COMPILE_MODEL, request);
    let result = response.result.expect("compileModel result");
    assert_eq!(result["success"], true, "{result:#}");
    assert_eq!(result["modules"], json!(["Models"]));
}

// ---- refusals ---------------------------------------------------------------------------------

fn assert_refused(response: morphir_extension_sdk::protocol::ExtensionResponse) {
    assert!(response.result.is_none(), "{:?}", response.result);
    assert!(response.error.is_some());
}

#[test]
fn generate_model_refuses_an_unagreed_ion_tuple() {
    let session = Session::on_model_line();
    let mut request = a_generate_model_request(compiled_model(&session));
    request.input = an_ion_tuple(ModelOperation::GenerateModel);
    assert_refused(session.call(methods::GENERATE_MODEL, request));
}

#[test]
fn generate_model_refuses_an_unagreed_ir_version() {
    let session = Session::on_model_line();
    let mut request = a_generate_model_request(compiled_model(&session));
    request.input.ir_version = "3.0.0".into();
    request.model.ir_version = "3.0.0".into();
    assert_refused(session.call(methods::GENERATE_MODEL, request));
}

#[test]
fn compile_model_refuses_an_unagreed_output_tuple() {
    let session = Session::on_model_line();
    let mut request = a_compile_model_request();
    request.output = an_ion_tuple(ModelOperation::CompileModel);
    assert_refused(session.call(methods::COMPILE_MODEL, request));
}

#[test]
fn generate_model_refuses_another_envelope_version() {
    let session = Session::on_model_line();
    let mut request = a_generate_model_request(compiled_model(&session));
    request.model.envelope_version = "0.1.0-draft.2".into();
    assert_refused(session.call(methods::GENERATE_MODEL, request));
}

#[test]
fn generate_model_refuses_a_payload_that_fails_its_digest() {
    let session = Session::on_model_line();
    let mut request = a_generate_model_request(compiled_model(&session));
    request.model.payload.sha256 = "0".repeat(64);
    assert_refused(session.call(methods::GENERATE_MODEL, request));
}

#[test]
fn model_operations_are_refused_on_the_0_1_line() {
    let session = Session::on_0_1();
    assert_refused(session.call(methods::COMPILE_MODEL, a_compile_model_request()));
}

#[test]
fn generate_model_refuses_a_required_guarantee() {
    let session = Session::on_model_line();
    let mut request = a_generate_model_request(compiled_model(&session));
    request.required_guarantees = vec![ModelGuaranteeRequirement {
        guarantee: "morphir.typed".into(),
        scope: "acme/example".into(),
    }];
    let response = session.call(methods::GENERATE_MODEL, request);
    let generated = response
        .result
        .map(|result| serde_json::from_value::<GenerateResult>(result).unwrap());
    assert!(
        generated.is_none_or(|result| !result.success && result.artifacts.is_empty()),
        "a required guarantee must not generate artifacts"
    );
}

// ---- the trait methods without a session ------------------------------------------------------

#[test]
fn the_trait_methods_refuse_an_unagreed_tuple_without_a_session() {
    let mut compile = a_compile_model_request();
    compile.output = an_ion_tuple(ModelOperation::CompileModel);
    assert!(RustExtension.compile_model(compile).is_err());

    let session = Session::on_model_line();
    let model = compiled_model(&session);
    let mut generate = a_generate_model_request(model.clone());
    generate.input = an_ion_tuple(ModelOperation::GenerateModel);
    generate.model.profile = "morphir-model".into();
    generate.model.payload.codec = ModelCodec::IonBinary;
    assert!(RustExtension.generate_model(generate).is_err());

    let mut generate = a_generate_model_request(model);
    generate.model.profile = "morphir-model".into();
    assert!(RustExtension.generate_model(generate).is_err());
}

#[test]
fn compile_model_refuses_an_ir_version_option_that_differs_from_the_tuple_without_a_session() {
    let mut request = a_compile_model_request();
    request.options.ir_version = "3".into();
    assert!(RustExtension.compile_model(request).is_err());
}

#[test]
fn the_trait_method_refuses_a_required_guarantee_without_a_session() {
    let session = Session::on_model_line();
    let mut request = a_generate_model_request(compiled_model(&session));
    request.required_guarantees = vec![ModelGuaranteeRequirement {
        guarantee: "morphir.typed".into(),
        scope: "acme/example".into(),
    }];
    let result = RustExtension.generate_model(request).unwrap();
    assert!(!result.success && result.artifacts.is_empty());
    assert_eq!(
        result.diagnostics[0].code.as_deref(),
        Some("RS_UNSUPPORTED_GUARANTEE")
    );
}

//! MEP `0.2.0-draft.1` model operations for one tuple per operation: profile `morphir-ir`, codec
//! `morphir-json`, envelope `0.1.0-draft.1` and IR `4.0.0`.
//!
//! Both operations reuse the MEP `0.1` compile and generate paths. The model payload is the IR v4
//! JSON document that `morphir.frontend.compile` returns. The SDK session checks tuples, limits and
//! payload integrity before and after each call; this module checks the tuple again because the
//! trait methods can also be called without a session.
//!
//! The extension offers no Ion tuple: the released guest is WASM, and the SDK has no Ion reader for
//! WASM targets.

use crate::RustExtension;
use morphir_extension_sdk::model::{
    CompileModelRequest, CompileModelResult, GenerateModelRequest, MODEL_ENVELOPE_VERSION,
    ModelCodec, ModelOperation, ModelPayload, ModelTransportLimits, ModelTransportSupport,
    ModelTransportTuple, MorphirModel, retain_model,
};
use morphir_extension_sdk::prelude::*;

/// The model-content profile that this extension reads and writes.
const PROFILE: &str = "morphir-ir";
/// The semantic IR version of the model payload.
const IR_VERSION: &str = "4.0.0";
/// The IR version option of the MEP `0.1` compile path that produces the payload.
const LEGACY_IR_VERSION: &str = "4";

/// The complete tuple of one operation.
fn tuple(operation: ModelOperation) -> ModelTransportTuple {
    ModelTransportTuple {
        operation,
        profile: PROFILE.into(),
        codec: ModelCodec::MorphirJson,
        envelope_version: MODEL_ENVELOPE_VERSION.into(),
        ir_version: IR_VERSION.into(),
    }
}

/// The guest's own upper bounds. A host takes the smaller value of each limit.
fn limits() -> ModelTransportLimits {
    ModelTransportLimits {
        max_encoded_bytes: 64 * 1024 * 1024,
        max_decoded_bytes: 48 * 1024 * 1024,
        max_depth: 1024,
        max_nodes: 16 * 1024 * 1024,
    }
}

/// The model transport that the extension advertises.
pub(crate) fn support() -> ModelTransportSupport {
    ModelTransportSupport {
        tuples: vec![
            tuple(ModelOperation::CompileModel),
            tuple(ModelOperation::GenerateModel),
        ],
        limits: limits(),
        guarantees: vec![],
    }
}

/// Compile one Rust document into a Morphir IR v4 model envelope.
pub(crate) fn compile_model(request: CompileModelRequest) -> Result<CompileModelResult> {
    if request.output != tuple(ModelOperation::CompileModel) {
        return Err(ExtensionError::InvalidParams(
            "Unsupported model output tuple".into(),
        ));
    }
    // The frontend does not compile against dependencies, so their models are never decoded.
    if !request.dependencies.is_empty() {
        return Ok(CompileModelResult::Failed {
            diagnostics: vec![crate::error(
                "RS_DEPENDENCIES",
                "External dependencies are not supported",
            )],
        });
    }
    let package_name = request.package.name.clone();
    let mut options = request.options;
    options.ir_version = LEGACY_IR_VERSION.into();
    let result = RustExtension.compile(CompileRequest {
        language_id: request.language_id,
        sources: request.sources,
        package: request.package,
        dependencies: vec![],
        options,
        baseline: None,
    })?;
    let Some(ir) = result.ir.filter(|_| result.success) else {
        return Ok(CompileModelResult::Failed {
            diagnostics: result.diagnostics,
        });
    };
    Ok(CompileModelResult::Compiled {
        model: Box::new(encode(&package_name, &ir)?),
        diagnostics: result.diagnostics,
        modules: result.modules,
    })
}

/// Generate Rust from a Morphir IR v4 model envelope.
pub(crate) fn generate_model(request: GenerateModelRequest) -> Result<GenerateResult> {
    let input = tuple(ModelOperation::GenerateModel);
    if request.input != input {
        return Err(ExtensionError::InvalidParams(
            "Unsupported model input tuple".into(),
        ));
    }
    if let Some(required) = request.required_guarantees.first() {
        return Ok(GenerateResult {
            success: false,
            artifacts: vec![],
            diagnostics: vec![crate::error(
                "RS_UNSUPPORTED_GUARANTEE",
                format!(
                    "The Rust backend cannot check guarantee '{}'",
                    required.guarantee
                ),
            )],
        });
    }
    // Admission checks the envelope against the tuple, the payload's length and SHA-256, and the
    // limits, before the bytes are read as IR.
    let retained = retain_model(&request.model, &input, &limits())
        .map_err(|error| ExtensionError::InvalidParams(format!("Model not admitted: {error}")))?;
    let ir = serde_json::from_slice(retained.bytes())?;
    RustExtension.generate(GenerateRequest {
        ir,
        target: request.target,
        options: request.options,
    })
}

fn encode(package_name: &str, ir: &serde_json::Value) -> Result<MorphirModel> {
    let bytes = serde_json::to_vec(ir)?;
    let payload = ModelPayload::from_bytes(ModelCodec::MorphirJson, &bytes)
        .map_err(|error| ExtensionError::ExecutionFailed(format!("Model payload: {error}")))?;
    Ok(MorphirModel {
        envelope_version: MODEL_ENVELOPE_VERSION.into(),
        model_id: package_name.into(),
        revision: format!("sha256:{}", payload.sha256),
        ir_version: IR_VERSION.into(),
        profile: PROFILE.into(),
        payload,
        evidence: vec![],
        critical: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_advertised_support_is_valid_for_this_target() {
        let support = support();
        let agreed =
            morphir_extension_sdk::model::NegotiatedModelTransport::select(&support, &support)
                .unwrap();
        assert_eq!(agreed.support(), &support);
        assert!(
            support
                .tuples
                .iter()
                .all(|tuple| tuple.codec == ModelCodec::MorphirJson)
        );
    }

    #[test]
    fn dependencies_fail_compilation_without_decoding_their_models() {
        let request: CompileModelRequest = serde_json::from_value(serde_json::json!({
            "languageId": "rust",
            "sources": {"documents": [
                {"uri": "models.rs", "languageId": "rust", "version": 1, "text": "pub type A = i64;"}
            ]},
            "package": {"name": "acme/example"},
            "dependencies": [{"packageName": "acme/other", "model": {
                "envelopeVersion": "0.1.0-draft.1", "modelId": "acme/other", "revision": "r",
                "irVersion": "4.0.0", "profile": "morphir-ir",
                "payload": {"codec": "morphir-json", "layout": "single-document",
                            "decodedByteLength": 1, "sha256": "x", "content": "not base64"}
            }}],
            "options": {"irVersion": "4.0.0", "typesOnly": false},
            "contextDigest": "0".repeat(64),
            "output": tuple(ModelOperation::CompileModel),
        }))
        .unwrap();
        let CompileModelResult::Failed { diagnostics } = compile_model(request).unwrap() else {
            panic!("dependencies must fail compilation");
        };
        assert_eq!(diagnostics[0].code.as_deref(), Some("RS_DEPENDENCIES"));
    }
}

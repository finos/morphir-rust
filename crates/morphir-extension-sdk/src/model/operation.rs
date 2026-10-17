//! Operation admission shared by guest and host, including generic-call paths.

use super::admission::{Budget, is_digest};
use super::*;
use crate::ExtensionCapabilities;
use serde_json::Value;

fn invalid() -> ModelAdmissionError {
    ModelAdmissionError::new("invalid_params")
}
fn selected(
    transport: &NegotiatedModelTransport,
    tuple: &ModelTransportTuple,
    operation: ModelOperation,
) -> Result<(), ModelAdmissionError> {
    if tuple.operation != operation || !transport.support().tuples.contains(tuple) {
        return Err(ModelAdmissionError::new("unsupported_tuple"));
    }
    Ok(())
}

/// Validate a model invocation before dispatch or transport I/O.
///
/// The default evidence policy refuses required guarantees even when claims
/// match: no built-in remote proof vocabulary or authority is defined yet.
/// Optional evidence remains retained and bounded. Consumers must perform their
/// semantic IR checks before their pure operation; transport is not typing.
pub fn validate_model_invocation(
    transport: &NegotiatedModelTransport,
    capabilities: &ExtensionCapabilities,
    method: &str,
    params: &Value,
) -> Result<(), ModelAdmissionError> {
    let mut budget = Budget::new(&transport.support().limits)?;
    match method {
        "morphir.frontend.compileModel" => {
            super::wire_budget::WireBudget::new(&transport.support().limits)
                .dependencies(params)?;
            let request: CompileModelRequest =
                serde::Deserialize::deserialize(params).map_err(|_| invalid())?;
            selected(transport, &request.output, ModelOperation::CompileModel)?;
            request.sources.source_paths().map_err(|_| invalid())?;
            if !is_digest(&request.context_digest)
                || request.options.ir_version != request.output.ir_version
            {
                return Err(invalid());
            }
            let frontend = capabilities.frontend.as_ref().ok_or_else(invalid)?;
            if !frontend
                .languages
                .iter()
                .any(|language| language.id == request.language_id)
                || !supports_ir(&frontend.ir_versions, &request.output.ir_version)
                || request.sources.documents.is_empty()
                || (!frontend.multi_document && request.sources.documents.len() != 1)
            {
                return Err(ModelAdmissionError::new("capability_unavailable"));
            }
            for dependency in &request.dependencies {
                if dependency.package_name.trim().is_empty() {
                    return Err(invalid());
                }
                budget.model(&dependency.model, &request.output)?;
            }
        }
        "morphir.backend.generateModel" => {
            super::wire_budget::WireBudget::new(&transport.support().limits)
                .model(&params["model"])?;
            let request: GenerateModelRequest =
                serde::Deserialize::deserialize(params).map_err(|_| invalid())?;
            selected(transport, &request.input, ModelOperation::GenerateModel)?;
            if !is_digest(&request.context_digest) {
                return Err(invalid());
            }
            let backend = capabilities.backend.as_ref().ok_or_else(invalid)?;
            if !backend.targets.contains(&request.target)
                || !supports_ir(&backend.ir_versions, &request.input.ir_version)
            {
                return Err(ModelAdmissionError::new("capability_unavailable"));
            }
            budget.model(&request.model, &request.input)?;
            if let Some(requirement) = request.required_guarantees.first() {
                if requirement.guarantee.trim().is_empty() || requirement.scope.trim().is_empty() {
                    return Err(invalid());
                }
                if !transport
                    .support()
                    .guarantees
                    .contains(&requirement.guarantee)
                {
                    return Err(ModelAdmissionError::new("unknown_required_guarantee"));
                }
                if !request.model.evidence.iter().any(|evidence| {
                    evidence.guarantee == requirement.guarantee
                        && evidence.model_id == request.model.model_id
                        && evidence.model_revision == request.model.revision
                        && evidence.context_digest == request.context_digest
                        && evidence.scope == requirement.scope
                }) {
                    return Err(ModelAdmissionError::new("missing_evidence"));
                }
                // Exact binding is necessary but not sufficient. Attribution is
                // not authentication and a hash cannot establish a guarantee.
                return Err(ModelAdmissionError::new("untrusted_evidence"));
            }
        }
        _ => return Err(ModelAdmissionError::new("unsupported_method")),
    }
    Ok(())
}

/// Check a rich operation's response against its immutable request selection.
/// Invalid successful compilation results cannot escape to the caller.
pub fn validate_model_result(
    transport: &NegotiatedModelTransport,
    method: &str,
    params: &Value,
    result: &Value,
) -> Result<(), ModelAdmissionError> {
    match method {
        "morphir.frontend.compileModel" => {
            let request: CompileModelRequest =
                serde::Deserialize::deserialize(params).map_err(|_| invalid())?;
            selected(transport, &request.output, ModelOperation::CompileModel)?;
            if result.get("success").and_then(Value::as_bool) == Some(true) {
                super::wire_budget::WireBudget::new(&transport.support().limits)
                    .model(&result["model"])?;
            }
            let result: CompileModelResult =
                serde::Deserialize::deserialize(result).map_err(|_| invalid())?;
            if let CompileModelResult::Compiled { model, .. } = result {
                retain_model(&model, &request.output, &transport.support().limits)?;
            }
        }
        "morphir.backend.generateModel" => {
            let _: crate::GenerateResult =
                serde::Deserialize::deserialize(result).map_err(|_| invalid())?;
        }
        _ => return Err(ModelAdmissionError::new("unsupported_method")),
    }
    Ok(())
}

// The old capability lists use major aliases such as "4". Rich tuples retain
// exact full versions; the alias denotes that major's baseline only.
fn supports_ir(versions: &[String], selected: &str) -> bool {
    let Ok(selected) = semver::Version::parse(selected) else {
        return false;
    };
    versions.iter().any(|version| {
        semver::Version::parse(version)
            .map(|version| version == selected)
            .unwrap_or_else(|_| {
                version.parse::<u32>().is_ok_and(|major| {
                    major >= 3
                        && version == &major.to_string()
                        && selected.major == u64::from(major)
                        && selected.minor == 0
                        && selected.patch == 0
                        && selected.pre.is_empty()
                        && selected.build.is_empty()
                })
            })
    })
}

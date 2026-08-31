//! Validation of untrusted response envelopes and initialization data.

use super::controller::NegotiatedSession;
use super::transport::ExpectedExtension;
use crate::extensions::protocol::{
    ExtensionResponse, InitializeResult, JSONRPC_VERSION, RpcError, methods,
};
use crate::{DaemonError, Result};
use morphir_distribution::RelativeArtifactPath;
use morphir_extension_sdk::{CompileRequest, CompileResult, ExtensionType, GenerateResult};
use std::collections::HashSet;

pub(super) enum ResponseFailure {
    Rpc(DaemonError),
    Invalid(DaemonError),
}

pub(super) fn validate_response(
    response: ExtensionResponse,
    expected_id: u64,
) -> std::result::Result<serde_json::Value, ResponseFailure> {
    if response.jsonrpc != JSONRPC_VERSION {
        return Err(ResponseFailure::Invalid(DaemonError::Extension(format!(
            "Extension response used unsupported JSON-RPC version '{}'",
            response.jsonrpc
        ))));
    }
    if response.id != expected_id {
        return Err(ResponseFailure::Invalid(DaemonError::Extension(format!(
            "Extension response ID {} did not match request ID {expected_id}",
            response.id
        ))));
    }
    match (response.result, response.error) {
        (Some(value), None) => Ok(value),
        (None, Some(RpcError { code, message, .. })) => Err(ResponseFailure::Rpc(
            DaemonError::Extension(format!("RPC error {code}: {message}")),
        )),
        _ => Err(ResponseFailure::Invalid(DaemonError::Extension(
            "Extension response must contain exactly one of result or error".into(),
        ))),
    }
}

pub(super) fn validate_method_result(
    method: &str,
    request_params: &serde_json::Value,
    value: serde_json::Value,
) -> Result<serde_json::Value> {
    if method == methods::GENERATE {
        return validate_generate_result(value);
    }
    if method == methods::COMPILE {
        let result: CompileResult = serde_json::from_value(value.clone())?;
        if result.success && result.ir_version.is_none() {
            return Err(DaemonError::Extension(
                "Successful compile result is missing irVersion".into(),
            ));
        }
        if result.success && result.ir.is_none() {
            return Err(DaemonError::Extension(
                "Successful compile result is missing ir".into(),
            ));
        }
        if result.success {
            let request: CompileRequest = serde_json::from_value(request_params.clone())?;
            let result_version = result
                .ir_version
                .as_deref()
                .expect("successful result version was validated");
            if result_version != request.options.ir_version {
                return Err(DaemonError::Extension(format!(
                    "Successful compile result irVersion '{result_version}' did not match requested irVersion '{}'",
                    request.options.ir_version
                )));
            }
            validate_compile_ir(
                result
                    .ir
                    .as_ref()
                    .expect("successful result IR was validated"),
                &request.options.ir_version,
            )?;
        }
    }
    Ok(value)
}

fn validate_generate_result(value: serde_json::Value) -> Result<serde_json::Value> {
    let mut result: GenerateResult = serde_json::from_value(value)?;
    for artifact in &mut result.artifacts {
        let path = RelativeArtifactPath::parse(artifact.path.clone()).map_err(|error| {
            DaemonError::Extension(format!(
                "Generated artifact path '{}' is invalid: {error}",
                artifact.path
            ))
        })?;
        artifact.path = path.as_str().to_owned();
    }
    serde_json::to_value(result).map_err(Into::into)
}

fn validate_compile_ir(ir: &serde_json::Value, requested_version: &str) -> Result<()> {
    if !matches!(requested_version, "3" | "4.0.0") {
        return Err(DaemonError::Extension(format!(
            "Successful compile result uses unsupported irVersion '{requested_version}' for host validation"
        )));
    }
    let attempted_root = ir.as_object().is_some_and(|object| {
        object.contains_key("formatVersion") || object.contains_key("distribution")
    });
    if attempted_root {
        let object = ir.as_object().expect("attempted IR root is an object");
        let format_version = object.get("formatVersion").ok_or_else(|| {
            DaemonError::Extension(
                "Successful compile result IR file is missing formatVersion".into(),
            )
        })?;
        if !object.contains_key("distribution") {
            return Err(DaemonError::Extension(
                "Successful compile result IR file is missing distribution".into(),
            ));
        }
        validate_embedded_format_version(format_version, requested_version)?;
        return validate_typed_ir_root(ir, requested_version);
    }
    validate_typed_raw_distribution(ir, requested_version)
}

fn validate_embedded_format_version(
    format_version: &serde_json::Value,
    requested_version: &str,
) -> Result<()> {
    let embedded_version = match format_version {
        serde_json::Value::String(version) => version.clone(),
        serde_json::Value::Number(version) if version.is_u64() => version.to_string(),
        _ => {
            return Err(DaemonError::Extension(
                "Successful compile result embedded formatVersion must be a string or non-negative integer"
                    .into(),
            ));
        }
    };
    if embedded_version != requested_version {
        return Err(DaemonError::Extension(format!(
            "Successful compile result embedded formatVersion '{embedded_version}' did not match requested irVersion '{requested_version}'"
        )));
    }
    Ok(())
}

fn validate_typed_ir_root(ir: &serde_json::Value, requested_version: &str) -> Result<()> {
    let result = match requested_version {
        "3" => serde_json::from_value::<morphir_core::ir::classic::Distribution>(ir.clone())
            .map(|_| ()),
        "4.0.0" => serde_json::from_value::<morphir_core::ir::v4::IRFile>(ir.clone()).map(|_| ()),
        _ => unreachable!("supported compile IR versions were checked"),
    };
    result.map_err(|error| {
        DaemonError::Extension(format!(
            "Successful compile result is not valid Morphir IR {requested_version}: {error}"
        ))
    })
}

fn validate_typed_raw_distribution(ir: &serde_json::Value, requested_version: &str) -> Result<()> {
    let result = match requested_version {
        "3" => serde_json::from_value::<morphir_core::ir::classic::DistributionBody>(ir.clone())
            .map(|_| ()),
        "4.0.0" => {
            serde_json::from_value::<morphir_core::ir::v4::Distribution>(ir.clone()).map(|_| ())
        }
        _ => unreachable!("supported compile IR versions were checked"),
    };
    result.map_err(|error| {
        DaemonError::Extension(format!(
            "Successful compile result is not valid Morphir IR {requested_version}: {error}"
        ))
    })
}

pub(in crate::extensions) fn validate_negotiation(
    expected: ExpectedExtension,
    offered_versions: &[String],
    result: InitializeResult,
) -> Result<NegotiatedSession> {
    let allows_legacy_backend = expected.discovered.is_some() && expected.capabilities.is_none();
    if !offered_versions.contains(&result.protocol_version) {
        return Err(DaemonError::Extension(format!(
            "Extension selected protocol version '{}' that the host did not offer",
            result.protocol_version
        )));
    }
    if result.extension.id != expected.id {
        return Err(DaemonError::Extension(format!(
            "Extension identity changed during initialization: expected '{}', initialized '{}'",
            expected.id, result.extension.id
        )));
    }
    let unique: HashSet<_> = result.extension.types.iter().copied().collect();
    if unique.len() != result.extension.types.len() {
        return Err(DaemonError::Extension(
            "Extension initialization repeated a capability kind".into(),
        ));
    }
    if let Some(discovered) = expected.discovered
        && (result.extension.version != discovered.version
            || result.extension.name != discovered.name
            || unique != discovered.types.iter().copied().collect())
    {
        return Err(DaemonError::Extension(format!(
            "Extension '{}' initialization metadata disagreed with discovery",
            expected.id
        )));
    }
    if let Some(discovered) = expected.capabilities
        && result.capabilities.backend != discovered.backend
    {
        return Err(DaemonError::Extension(format!(
            "Extension '{}' backend capabilities disagreed with discovery",
            expected.id
        )));
    }
    if unique.contains(&ExtensionType::Frontend) && result.capabilities.frontend.is_none() {
        return Err(DaemonError::Extension(
            "Extension declared Frontend without frontend capabilities".into(),
        ));
    }
    if !unique.contains(&ExtensionType::Frontend) && result.capabilities.frontend.is_some() {
        return Err(DaemonError::Extension(
            "Extension advertised frontend capabilities without declaring Frontend".into(),
        ));
    }
    let legacy_backend = allows_legacy_backend
        && unique.contains(&ExtensionType::Backend)
        && result.capabilities.backend.is_none();
    if unique.contains(&ExtensionType::Backend)
        && result.capabilities.backend.is_none()
        && !legacy_backend
    {
        return Err(DaemonError::Extension(
            "Extension declared Backend without backend capabilities".into(),
        ));
    }
    if !unique.contains(&ExtensionType::Backend) && result.capabilities.backend.is_some() {
        return Err(DaemonError::Extension(
            "Extension advertised backend capabilities without declaring Backend".into(),
        ));
    }
    Ok(NegotiatedSession {
        protocol_version: result.protocol_version,
        extension: result.extension,
        capabilities: result.capabilities,
        legacy_backend,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_request(ir_version: &str) -> serde_json::Value {
        serde_json::json!({
            "languageId": "elm",
            "documents": [],
            "package": {"name": "example/package", "exposedModules": []},
            "dependencies": [],
            "options": {"typesOnly": false, "irVersion": ir_version}
        })
    }

    fn successful_result(ir_version: &str, ir: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "success": true,
            "irVersion": ir_version,
            "ir": ir,
            "diagnostics": [],
            "modules": []
        })
    }

    fn v4_library_distribution() -> serde_json::Value {
        serde_json::json!({
            "Library": {
                "packageName": "example/package",
                "dependencies": {},
                "def": {"modules": {}}
            }
        })
    }

    #[test]
    fn rejects_unsafe_generated_artifact_paths() {
        for path in [
            "/tmp/schema.avsc",
            "../../.ssh/authorized_keys",
            "nested/../schema.avsc",
            r"C:\\Users\\Public\\schema.avsc",
        ] {
            let error = validate_method_result(
                methods::GENERATE,
                &serde_json::json!({}),
                serde_json::json!({
                    "success": true,
                    "artifacts": [{"path": path, "content": "{}"}],
                    "diagnostics": []
                }),
            )
            .expect_err("unsafe generated artifact paths must fail validation");

            assert!(
                error.to_string().contains("artifact path"),
                "unexpected error for {path}: {error}"
            );
        }
    }

    #[test]
    fn rejects_an_embedded_ir_format_version_that_differs_from_the_request() {
        let error = validate_method_result(
            methods::COMPILE,
            &compile_request("3"),
            successful_result(
                "3",
                serde_json::json!({
                    "formatVersion": "4.0.0",
                    "distribution": {"Library": {}}
                }),
            ),
        )
        .expect_err("embedded format mismatch should fail");

        assert!(error.to_string().contains("embedded formatVersion"));
    }

    #[test]
    fn rejects_an_ir_file_shape_without_an_embedded_format_version() {
        let error = validate_method_result(
            methods::COMPILE,
            &compile_request("4.0.0"),
            successful_result(
                "4.0.0",
                serde_json::json!({"distribution": {"Library": {}}}),
            ),
        )
        .expect_err("IR file shape without formatVersion should fail");

        assert!(error.to_string().contains("missing formatVersion"));
    }

    #[test]
    fn rejects_an_empty_object_as_untyped_compile_ir() {
        let error = validate_method_result(
            methods::COMPILE,
            &compile_request("3"),
            successful_result("3", serde_json::json!({})),
        )
        .expect_err("empty object is not typed Morphir IR");

        assert!(error.to_string().contains("valid Morphir IR"));
    }

    #[test]
    fn rejects_untyped_malformed_and_incomplete_compile_ir_shapes() {
        let invalid = [
            ("3", serde_json::Value::Null),
            ("3", serde_json::json!(true)),
            ("3", serde_json::json!(42)),
            ("3", serde_json::json!("not-ir")),
            ("3", serde_json::json!([])),
            ("3", serde_json::json!(["Library"])),
            ("3", serde_json::json!({"unrelated": {}})),
            ("3", serde_json::json!({"formatVersion": 3})),
            (
                "3",
                serde_json::json!({
                    "distribution": ["Library", [], [], {"modules": []}]
                }),
            ),
            ("4.0.0", serde_json::json!({"Library": {}})),
            (
                "4.0.0",
                serde_json::json!(["Library", [], [], {"modules": []}]),
            ),
        ];

        for (version, ir) in invalid {
            assert!(
                validate_method_result(
                    methods::COMPILE,
                    &compile_request(version),
                    successful_result(version, ir.clone()),
                )
                .is_err(),
                "{ir} must not validate as Morphir IR {version}"
            );
        }
    }

    #[test]
    fn accepts_numeric_classic_format_version_for_string_request_version() {
        validate_method_result(
            methods::COMPILE,
            &compile_request("3"),
            successful_result(
                "3",
                serde_json::json!({
                    "formatVersion": 3,
                    "distribution": ["Library", [], [], {"modules": []}]
                }),
            ),
        )
        .expect("numeric classic format should match string request version");
    }

    #[test]
    fn accepts_a_raw_classic_v3_library_distribution_body() {
        validate_method_result(
            methods::COMPILE,
            &compile_request("3"),
            successful_result("3", serde_json::json!(["Library", [], [], {"modules": []}])),
        )
        .expect("raw Classic V3 Library body is MEP-compatible");
    }

    #[test]
    fn accepts_string_v4_format_version() {
        validate_method_result(
            methods::COMPILE,
            &compile_request("4.0.0"),
            successful_result(
                "4.0.0",
                serde_json::json!({
                    "formatVersion": "4.0.0",
                    "distribution": v4_library_distribution()
                }),
            ),
        )
        .expect("string V4 format should match request version");
    }

    #[test]
    fn accepts_a_raw_distribution_without_an_ir_file_format_version() {
        validate_method_result(
            methods::COMPILE,
            &compile_request("4.0.0"),
            successful_result("4.0.0", v4_library_distribution()),
        )
        .expect("raw distributions are permitted without an IR file wrapper");
    }

    #[test]
    fn rejects_an_unknown_compile_ir_version_that_the_host_cannot_validate() {
        let error = validate_method_result(
            methods::COMPILE,
            &compile_request("5.0.0"),
            successful_result("5.0.0", v4_library_distribution()),
        )
        .expect_err("unknown IR versions cannot be schema-validated");

        assert!(error.to_string().contains("unsupported irVersion"));
    }
}

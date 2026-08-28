//! Validation of untrusted response envelopes and initialization data.

use super::controller::NegotiatedSession;
use super::transport::ExpectedExtension;
use crate::extensions::protocol::{
    ExtensionResponse, InitializeResult, JSONRPC_VERSION, RpcError, methods,
};
use crate::{DaemonError, Result};
use morphir_extension_sdk::{CompileResult, ExtensionType};
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
    value: serde_json::Value,
) -> Result<serde_json::Value> {
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
    }
    Ok(value)
}

pub(super) fn validate_negotiation(
    expected: ExpectedExtension,
    offered_versions: &[String],
    result: InitializeResult,
) -> Result<NegotiatedSession> {
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
    Ok(NegotiatedSession {
        protocol_version: result.protocol_version,
        extension: result.extension,
        capabilities: result.capabilities,
    })
}

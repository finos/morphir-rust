//! Snapshot negotiated guest support without changing legacy advertisements.

use super::*;
use crate::protocol::{InitializeParams, InitializeResult, MEP_VERSION};
use crate::{ExtensionCapabilities, ExtensionError, ExtensionInfo};

pub(crate) fn versions(capabilities: &ExtensionCapabilities) -> Vec<String> {
    let mut versions = vec![MEP_VERSION.to_owned()];
    if capabilities
        .model_transport
        .as_ref()
        .is_some_and(|support| {
            !support.tuples.is_empty() && NegotiatedModelTransport::select(support, support).is_ok()
        })
    {
        versions.insert(0, MODEL_MEP_VERSION.to_owned());
    }
    versions
}

pub(crate) fn initialize(
    params: &InitializeParams,
    info: &ExtensionInfo,
    capabilities: &ExtensionCapabilities,
) -> crate::Result<InitializeResult> {
    let mut implemented = versions(capabilities);
    if params.model_transport.is_none() {
        implemented.retain(|version| version != MODEL_MEP_VERSION);
    }
    let selected = implemented
        .iter()
        .find(|version| params.protocol_versions.contains(version))
        .ok_or_else(|| ExtensionError::ProtocolVersionMismatch {
            host_versions: params.protocol_versions.clone(),
            extension_versions: implemented.clone(),
        })?;
    let mut capabilities = capabilities.clone();
    capabilities.model_transport = if selected == MODEL_MEP_VERSION {
        match (&params.model_transport, &capabilities.model_transport) {
            (Some(host), Some(guest)) => Some(
                NegotiatedModelTransport::select(host, guest)
                    .map_err(|_| {
                        ExtensionError::InvalidParams("Invalid model transport offer".into())
                    })?
                    .support()
                    .clone(),
            ),
            _ => None,
        }
    } else {
        None
    };
    Ok(InitializeResult {
        protocol_version: selected.clone(),
        extension: info.clone(),
        capabilities,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_reader_claim_never_advertises_the_model_draft() {
        let capabilities = ExtensionCapabilities {
            model_transport: Some(ModelTransportSupport {
                tuples: vec![ModelTransportTuple {
                    operation: ModelOperation::GenerateModel,
                    profile: "future-model".into(),
                    codec: ModelCodec::MorphirJson,
                    envelope_version: MODEL_ENVELOPE_VERSION.into(),
                    ir_version: "4.0.0".into(),
                }],
                limits: ModelTransportLimits {
                    max_encoded_bytes: 1024,
                    max_decoded_bytes: 768,
                    max_depth: 32,
                    max_nodes: 128,
                },
                guarantees: vec![],
            }),
            ..Default::default()
        };
        assert_eq!(versions(&capabilities), vec!["0.1"]);
    }
}

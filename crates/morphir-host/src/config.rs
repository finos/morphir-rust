//! How a client introduces itself to a guest.

use morphir_extension_sdk::model::{MODEL_MEP_VERSION, ModelTransportSupport};
use morphir_extension_sdk::protocol::{InitializeParams, PeerInfo, SUPPORTED_MEP_VERSIONS};

/// How a client introduces itself to a guest, and which MEP versions it offers.
///
/// The client supplies the peer. The host library never names a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostConfig {
    peer: PeerInfo,
    protocol_versions: Vec<String>,
    model_transport: Option<ModelTransportSupport>,
}

impl HostConfig {
    /// A host with the default legacy MEP offer. Rich transport requires opt-in.
    pub fn new(peer: PeerInfo) -> Self {
        Self {
            peer,
            protocol_versions: SUPPORTED_MEP_VERSIONS
                .iter()
                .map(|version| (*version).to_owned())
                .collect(),
            model_transport: None,
        }
    }

    /// A host that offers exactly `protocol_versions`, in that order.
    ///
    /// For callers that already hold a version list from their own caller,
    /// such as the daemon's describe probe. Other clients use [`Self::new`].
    #[doc(hidden)]
    pub fn with_versions(peer: PeerInfo, protocol_versions: Vec<String>) -> Self {
        Self {
            peer,
            protocol_versions,
            model_transport: None,
        }
    }

    /// Opt into the exact model-operation draft with an explicit transport offer.
    ///
    /// The legacy versions remain available. The handshake validates the offer
    /// and the guest's selection; merely constructing this config grants no call.
    pub fn with_model_transport(mut self, offer: ModelTransportSupport) -> Self {
        self.protocol_versions
            .retain(|version| version != MODEL_MEP_VERSION);
        self.protocol_versions
            .insert(0, MODEL_MEP_VERSION.to_owned());
        self.model_transport = Some(offer);
        self
    }

    /// The client's name and version.
    pub fn peer(&self) -> &PeerInfo {
        &self.peer
    }

    /// The MEP versions the host offers, in canonical form such as `"0.1"`.
    pub fn protocol_versions(&self) -> &[String] {
        &self.protocol_versions
    }

    /// Parameters for `morphir.initialize` and `morphir.extension.describe`.
    pub fn initialize_params(&self) -> InitializeParams {
        InitializeParams {
            protocol_versions: self.protocol_versions.clone(),
            host: self.peer.clone(),
            model_transport: self.model_transport.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_extension_sdk::protocol::{PeerInfo, PeerKind, SUPPORTED_MEP_VERSIONS};

    fn peer() -> PeerInfo {
        PeerInfo {
            kind: PeerKind::Unspecified,
            name: "test-host".into(),
            version: "1.2.3".into(),
        }
    }

    fn supported() -> Vec<String> {
        SUPPORTED_MEP_VERSIONS
            .iter()
            .map(|version| (*version).to_owned())
            .collect()
    }

    #[test]
    fn offers_every_supported_version() {
        let config = HostConfig::new(peer());
        assert_eq!(config.protocol_versions(), supported().as_slice());
    }

    #[test]
    fn with_versions_offers_exactly_the_given_list() {
        let versions = vec!["0.1".to_owned(), "future".to_owned()];
        let config = HostConfig::with_versions(peer(), versions.clone());
        assert_eq!(config.protocol_versions(), versions.as_slice());
        assert_eq!(config.initialize_params().protocol_versions, versions);
    }

    #[test]
    fn initialize_params_carry_the_client_peer() {
        let params = HostConfig::new(peer()).initialize_params();
        assert_eq!(params.host, peer());
        assert_eq!(params.protocol_versions, supported());
    }
}

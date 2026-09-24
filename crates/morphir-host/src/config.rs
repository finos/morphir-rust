//! How a client introduces itself to a guest.

use morphir_extension_sdk::protocol::{InitializeParams, PeerInfo, SUPPORTED_MEP_VERSIONS};

/// How a client introduces itself to a guest, and which MEP versions it offers.
///
/// The client supplies the peer. The host library never names a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostConfig {
    peer: PeerInfo,
    protocol_versions: Vec<String>,
}

impl HostConfig {
    /// A host that offers every MEP version this SDK supports.
    pub fn new(peer: PeerInfo) -> Self {
        Self {
            peer,
            protocol_versions: SUPPORTED_MEP_VERSIONS
                .iter()
                .map(|version| (*version).to_owned())
                .collect(),
        }
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
    fn initialize_params_carry_the_client_peer() {
        let params = HostConfig::new(peer()).initialize_params();
        assert_eq!(params.host, peer());
        assert_eq!(params.protocol_versions, supported());
    }
}

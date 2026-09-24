//! Validated negotiation data.

use morphir_extension_sdk::protocol::methods;
use morphir_extension_sdk::{ExtensionCapabilities, ExtensionInfo, ExtensionType};
use morphir_workspace::{DiscoveryRequest, speaks_workspace_discovery_protocol};

/// Validated application data produced by MEP negotiation.
#[derive(Debug, Clone)]
pub struct Negotiated {
    protocol_version: String,
    extension: ExtensionInfo,
    capabilities: ExtensionCapabilities,
    legacy_backend: bool,
}

impl Negotiated {
    /// Record a negotiation that the client's checks accepted.
    ///
    /// `legacy_backend` is true only for a schema v1 backend that may generate
    /// without a typed backend capability.
    pub fn new(
        protocol_version: String,
        extension: ExtensionInfo,
        capabilities: ExtensionCapabilities,
        legacy_backend: bool,
    ) -> Self {
        Self {
            protocol_version,
            extension,
            capabilities,
            legacy_backend,
        }
    }

    /// Selected MEP version.
    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
    }

    /// Validated extension identity and capability kinds.
    pub fn extension(&self) -> &ExtensionInfo {
        &self.extension
    }

    /// Features negotiated for this session.
    pub fn capabilities(&self) -> &ExtensionCapabilities {
        &self.capabilities
    }

    /// Whether the guest advertised the capability that `method` needs.
    pub fn supports_method(&self, method: &str) -> bool {
        match method {
            methods::COMPILE => {
                self.extension.types.contains(&ExtensionType::Frontend)
                    && self
                        .capabilities
                        .frontend
                        .as_ref()
                        .is_some_and(|frontend| frontend.compile)
            }
            methods::GENERATE => {
                self.extension.types.contains(&ExtensionType::Backend)
                    && (self.legacy_backend
                        || self
                            .capabilities
                            .backend
                            .as_ref()
                            .is_some_and(|backend| backend.generate))
            }
            methods::VALIDATE => self.extension.types.contains(&ExtensionType::Validator),
            methods::TRANSFORM => self.extension.types.contains(&ExtensionType::Transform),
            methods::WORKSPACE_DISCOVER => {
                self.extension.types.contains(&ExtensionType::Workspace)
                    && self
                        .capabilities
                        .workspace
                        .as_ref()
                        .is_some_and(|workspace| {
                            workspace.discover
                                && workspace
                                    .protocol_versions
                                    .iter()
                                    .any(speaks_workspace_discovery_protocol)
                        })
            }
            _ => true,
        }
    }

    /// Whether the guest supports this particular request of `method`.
    pub fn supports_invocation(&self, method: &str, params: &serde_json::Value) -> bool {
        if method != methods::WORKSPACE_DISCOVER {
            return true;
        }
        let Some(workspace) = self.capabilities.workspace.as_ref() else {
            return false;
        };
        serde_json::from_value::<DiscoveryRequest>(params.clone())
            .ok()
            .is_some_and(|request| {
                speaks_workspace_discovery_protocol(&request.protocol_version)
                    && workspace
                        .protocol_versions
                        .contains(&request.protocol_version)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_extension_sdk::protocol::methods;
    use morphir_extension_sdk::{
        ExtensionCapabilities, ExtensionInfo, ExtensionType, FrontendCapability,
    };

    fn info(types: Vec<ExtensionType>) -> ExtensionInfo {
        ExtensionInfo {
            id: "guest".into(),
            name: "Guest".into(),
            types,
            ..ExtensionInfo::default()
        }
    }

    #[test]
    fn compile_needs_a_frontend_that_enables_compile() {
        let capabilities = ExtensionCapabilities {
            frontend: Some(FrontendCapability {
                compile: true,
                ..FrontendCapability::default()
            }),
            ..ExtensionCapabilities::default()
        };
        let frontend = Negotiated::new(
            "0.1".into(),
            info(vec![ExtensionType::Frontend]),
            capabilities.clone(),
            false,
        );
        assert!(frontend.supports_method(methods::COMPILE));
        let backend = Negotiated::new(
            "0.1".into(),
            info(vec![ExtensionType::Backend]),
            capabilities,
            false,
        );
        assert!(!backend.supports_method(methods::COMPILE));
    }

    #[test]
    fn a_legacy_backend_may_generate_without_a_backend_capability() {
        let legacy = Negotiated::new(
            "0.1".into(),
            info(vec![ExtensionType::Backend]),
            ExtensionCapabilities::default(),
            true,
        );
        assert!(legacy.supports_method(methods::GENERATE));
        let strict = Negotiated::new(
            "0.1".into(),
            info(vec![ExtensionType::Backend]),
            ExtensionCapabilities::default(),
            false,
        );
        assert!(!strict.supports_method(methods::GENERATE));
    }
}

//! What a host expects of a guest, and the rules that check the guest against it.
//!
//! [`ExpectedExtension`] records identity and capability metadata known
//! before negotiation, for example from extension discovery or an installed
//! catalog record. [`ExpectedChecks`] runs the full negotiation rules against
//! it: the offered version, identity, the discovery lock on metadata and
//! capabilities, kind and capability consistency, and the schema v1 legacy
//! backend exemption.

use crate::{HostError, Negotiated, SessionChecks};
use morphir_extension_sdk::protocol::InitializeResult;
use morphir_extension_sdk::{
    BackendCapability, ExtensionCapabilities, ExtensionInfo, ExtensionType, FrontendCapability,
};
use std::collections::HashSet;

/// Capability members persisted by installed-extension metadata.
///
/// The distribution manifest currently persists frontend and backend metadata,
/// but not every member of [`ExtensionCapabilities`]. Negotiation locks these
/// persisted members while validating other declared members normally. A
/// `None` member was not persisted and remains unlocked during negotiation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersistedExtensionCapabilities {
    frontend: Option<FrontendCapability>,
    backend: Option<BackendCapability>,
}

impl PersistedExtensionCapabilities {
    /// Create a persisted capability expectation from its stored members.
    pub fn new(frontend: Option<FrontendCapability>, backend: Option<BackendCapability>) -> Self {
        Self { frontend, backend }
    }

    /// Return the persisted frontend member, when present.
    pub fn frontend(&self) -> Option<&FrontendCapability> {
        self.frontend.as_ref()
    }

    /// Return the persisted backend member, when present.
    pub fn backend(&self) -> Option<&BackendCapability> {
        self.backend.as_ref()
    }

    /// Whether no member was persisted.
    pub fn is_empty(&self) -> bool {
        self.frontend.is_none() && self.backend.is_none()
    }
}

/// Capability metadata known before negotiation.
#[derive(Debug, Clone)]
pub enum CapabilityExpectation {
    /// Every advertised capability member is known and must match.
    Exact(ExtensionCapabilities),
    /// Persisted frontend and backend members must match; other members remain negotiable.
    Persisted(PersistedExtensionCapabilities),
    /// Only the backend member was persisted and must match.
    Backend(BackendCapability),
}

/// Identity known before protocol negotiation.
#[derive(Debug, Clone)]
pub struct ExpectedExtension {
    id: String,
    discovered: Option<ExtensionInfo>,
    capabilities: Option<CapabilityExpectation>,
    allows_legacy_backend: bool,
}

impl ExpectedExtension {
    /// Expect only a stable extension identifier.
    pub fn identified(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            discovered: None,
            capabilities: None,
            allows_legacy_backend: false,
        }
    }

    /// Require initialization metadata to agree with discovery metadata.
    pub fn discovered(info: ExtensionInfo) -> Self {
        Self {
            id: info.id.clone(),
            discovered: Some(info),
            capabilities: None,
            allows_legacy_backend: false,
        }
    }

    /// Preserve schema-v1 backend behavior for installed legacy metadata.
    ///
    /// For hosts that restore a catalog record written by an older CLI.
    #[doc(hidden)]
    pub fn legacy_discovered(info: ExtensionInfo) -> Self {
        Self {
            id: info.id.clone(),
            discovered: Some(info),
            capabilities: None,
            allows_legacy_backend: true,
        }
    }

    /// Require initialization metadata and every capability to agree with discovery.
    pub fn discovered_with_capabilities(
        info: ExtensionInfo,
        capabilities: ExtensionCapabilities,
    ) -> Self {
        Self {
            id: info.id.clone(),
            discovered: Some(info),
            capabilities: Some(CapabilityExpectation::Exact(capabilities)),
            allows_legacy_backend: false,
        }
    }

    /// Require persisted frontend and backend metadata to agree with discovery.
    ///
    /// Capability members not represented by installed metadata remain
    /// negotiable and are still checked against the extension's declared types.
    pub fn discovered_with_persisted_capabilities(
        info: ExtensionInfo,
        capabilities: PersistedExtensionCapabilities,
    ) -> Self {
        Self {
            id: info.id.clone(),
            discovered: Some(info),
            capabilities: Some(CapabilityExpectation::Persisted(capabilities)),
            allows_legacy_backend: false,
        }
    }

    /// Require exact backend metadata while leaving unpersisted members negotiable.
    pub fn discovered_with_backend_capability(
        info: ExtensionInfo,
        backend: BackendCapability,
    ) -> Self {
        Self {
            id: info.id.clone(),
            discovered: Some(info),
            capabilities: Some(CapabilityExpectation::Backend(backend)),
            allows_legacy_backend: false,
        }
    }

    /// Require initialization metadata to agree with discovery, under a
    /// capability expectation the caller already built.
    ///
    /// For hosts that restore a catalog record written by an older CLI.
    #[doc(hidden)]
    pub fn discovered_with_expectation(
        info: ExtensionInfo,
        capabilities: CapabilityExpectation,
    ) -> Self {
        Self {
            id: info.id.clone(),
            discovered: Some(info),
            capabilities: Some(capabilities),
            allows_legacy_backend: false,
        }
    }

    /// Return the stable extension identifier known before negotiation.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Return exact identity metadata obtained during discovery, when available.
    pub fn extension_info(&self) -> Option<&ExtensionInfo> {
        self.discovered.as_ref()
    }

    /// Return complete discovery capabilities, when every member is locked.
    pub fn capabilities(&self) -> Option<&ExtensionCapabilities> {
        match self.capabilities.as_ref() {
            Some(CapabilityExpectation::Exact(capabilities)) => Some(capabilities),
            Some(CapabilityExpectation::Persisted(_))
            | Some(CapabilityExpectation::Backend(_))
            | None => None,
        }
    }

    /// Return persisted installed capability members, when those members are locked.
    pub fn persisted_capabilities(&self) -> Option<&PersistedExtensionCapabilities> {
        match self.capabilities.as_ref() {
            Some(CapabilityExpectation::Persisted(capabilities)) => Some(capabilities),
            Some(CapabilityExpectation::Exact(_))
            | Some(CapabilityExpectation::Backend(_))
            | None => None,
        }
    }

    /// Return the locked frontend capability, whether the lock is exact or persisted.
    pub fn frontend_capability(&self) -> Option<&FrontendCapability> {
        match self.capabilities.as_ref() {
            Some(CapabilityExpectation::Exact(capabilities)) => capabilities.frontend.as_ref(),
            Some(CapabilityExpectation::Persisted(capabilities)) => capabilities.frontend(),
            Some(CapabilityExpectation::Backend(_)) | None => None,
        }
    }

    /// Return the locked backend capability, whether the lock is exact or partial.
    pub fn backend_capability(&self) -> Option<&BackendCapability> {
        match self.capabilities.as_ref() {
            Some(CapabilityExpectation::Exact(capabilities)) => capabilities.backend.as_ref(),
            Some(CapabilityExpectation::Persisted(capabilities)) => capabilities.backend(),
            Some(CapabilityExpectation::Backend(backend)) => Some(backend),
            None => None,
        }
    }

    /// Whether a schema v1 backend may generate without a typed backend capability.
    pub fn allows_legacy_backend(&self) -> bool {
        self.allows_legacy_backend
    }

    /// What the host locked about the guest's capabilities, if anything.
    pub fn expectation(&self) -> Option<&CapabilityExpectation> {
        self.capabilities.as_ref()
    }
}

/// The frontend members that differ, named as they are spelled on the wire.
///
/// A missing record on either side is one difference, not five: the host
/// discovered a frontend the guest does not advertise, or the other way round.
fn frontend_differences(
    advertised: Option<&FrontendCapability>,
    discovered: Option<&FrontendCapability>,
) -> Vec<&'static str> {
    match (advertised, discovered) {
        (Some(advertised), Some(discovered)) => {
            let mut members = Vec::new();
            if advertised.languages != discovered.languages {
                members.push("languages");
            }
            if advertised.ir_versions != discovered.ir_versions {
                members.push("irVersions");
            }
            if advertised.compile != discovered.compile {
                members.push("compile");
            }
            if advertised.incremental != discovered.incremental {
                members.push("incremental");
            }
            if advertised.fragments != discovered.fragments {
                members.push("fragments");
            }
            if advertised.multi_document != discovered.multi_document {
                members.push("multiDocument");
            }
            members
        }
        (None, Some(_)) => vec!["no frontend capability was advertised"],
        (Some(_), None) => vec!["no frontend capability was discovered"],
        (None, None) => Vec::new(),
    }
}

/// A persisted frontend record, completed with the members it cannot carry.
///
/// An installed record persists the frontend members its release record
/// declares. `multiDocument` is not one of them, so a persisted record's value
/// for it is unknown rather than false, and the guest's advertised value
/// stands. Every member the record does carry must still agree.
fn persisted_frontend(
    persisted: &FrontendCapability,
    advertised: Option<&FrontendCapability>,
) -> FrontendCapability {
    FrontendCapability {
        multi_document: advertised.is_some_and(|advertised| advertised.multi_document),
        ..persisted.clone()
    }
}

/// The backend members that differ, named as they are spelled on the wire.
fn backend_differences(
    advertised: Option<&BackendCapability>,
    discovered: Option<&BackendCapability>,
) -> Vec<&'static str> {
    match (advertised, discovered) {
        (Some(advertised), Some(discovered)) => {
            let mut members = Vec::new();
            if advertised.targets != discovered.targets {
                members.push("targets");
            }
            if advertised.ir_versions != discovered.ir_versions {
                members.push("irVersions");
            }
            if advertised.generate != discovered.generate {
                members.push("generate");
            }
            members
        }
        (None, Some(_)) => vec!["no backend capability was advertised"],
        (Some(_), None) => vec!["no backend capability was discovered"],
        (None, None) => Vec::new(),
    }
}

/// Check a guest's `morphir.initialize` answer against what the host expects of it.
pub fn validate_negotiation(
    expected: ExpectedExtension,
    offered_versions: &[String],
    result: InitializeResult,
) -> Result<Negotiated, HostError> {
    let allows_legacy_backend = expected.allows_legacy_backend;
    if !offered_versions.contains(&result.protocol_version) {
        return Err(HostError::VersionNotOffered {
            selected: result.protocol_version,
            offered: offered_versions.to_vec(),
        });
    }
    if result.extension.id != expected.id {
        return Err(HostError::Invalid(format!(
            "Extension identity changed during initialization: expected '{}', initialized '{}'",
            expected.id, result.extension.id
        )));
    }
    let unique: HashSet<_> = result.extension.types.iter().copied().collect();
    if unique.len() != result.extension.types.len() {
        return Err(HostError::Invalid(
            "Extension initialization repeated a capability kind".into(),
        ));
    }
    // The display name is presentation metadata. A repository record may
    // spell it differently from the guest (for example "Morphir Openapi"
    // derived from the identifier against the guest's "Morphir OpenAPI"), so
    // only the version and the capability kinds are held to discovery.
    if let Some(discovered) = expected.discovered {
        if result.extension.version != discovered.version {
            return Err(HostError::Invalid(format!(
                "Extension '{}' initialization metadata disagreed with discovery: version '{}' was discovered as '{}'",
                expected.id, result.extension.version, discovered.version
            )));
        }
        if unique != discovered.types.iter().copied().collect() {
            return Err(HostError::Invalid(format!(
                "Extension '{}' initialization metadata disagreed with discovery: capability kinds changed",
                expected.id
            )));
        }
    }
    if let Some(discovered) = expected.capabilities {
        // A mismatch here stops the session, so the message names the members
        // that differ: "frontend capabilities disagreed with discovery" alone
        // leaves a reader comparing two structures by hand.
        let mismatch = match discovered {
            CapabilityExpectation::Exact(discovered) if result.capabilities != discovered => {
                let backend = backend_differences(
                    result.capabilities.backend.as_ref(),
                    discovered.backend.as_ref(),
                );
                if backend.is_empty() {
                    Some((
                        "capabilities",
                        frontend_differences(
                            result.capabilities.frontend.as_ref(),
                            discovered.frontend.as_ref(),
                        ),
                    ))
                } else {
                    Some(("backend capabilities", backend))
                }
            }
            CapabilityExpectation::Backend(discovered)
                if result.capabilities.backend.as_ref() != Some(&discovered) =>
            {
                Some((
                    "backend capabilities",
                    backend_differences(result.capabilities.backend.as_ref(), Some(&discovered)),
                ))
            }
            CapabilityExpectation::Persisted(discovered)
                if discovered.frontend().is_some_and(|expected| {
                    result.capabilities.frontend.as_ref()
                        != Some(&persisted_frontend(
                            expected,
                            result.capabilities.frontend.as_ref(),
                        ))
                }) =>
            {
                let expected = discovered.frontend().map(|expected| {
                    persisted_frontend(expected, result.capabilities.frontend.as_ref())
                });
                Some((
                    "frontend capabilities",
                    frontend_differences(result.capabilities.frontend.as_ref(), expected.as_ref()),
                ))
            }
            CapabilityExpectation::Persisted(discovered)
                if discovered.backend().is_some_and(|expected| {
                    result.capabilities.backend.as_ref() != Some(expected)
                }) =>
            {
                Some((
                    "backend capabilities",
                    backend_differences(result.capabilities.backend.as_ref(), discovered.backend()),
                ))
            }
            CapabilityExpectation::Exact(_)
            | CapabilityExpectation::Persisted(_)
            | CapabilityExpectation::Backend(_) => None,
        };
        if let Some((capability_scope, differences)) = mismatch {
            // An `Exact` expectation also covers members outside the frontend
            // and backend records, so the list can be empty.
            let named = if differences.is_empty() {
                String::new()
            } else {
                format!(": {}", differences.join(", "))
            };
            return Err(HostError::Invalid(format!(
                "Extension '{}' {capability_scope} disagreed with discovery{named}",
                expected.id
            )));
        }
    }
    if unique.contains(&ExtensionType::Frontend) && result.capabilities.frontend.is_none() {
        return Err(HostError::Invalid(
            "Extension declared Frontend without frontend capabilities".into(),
        ));
    }
    if !unique.contains(&ExtensionType::Frontend) && result.capabilities.frontend.is_some() {
        return Err(HostError::Invalid(
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
        return Err(HostError::Invalid(
            "Extension declared Backend without backend capabilities".into(),
        ));
    }
    if !unique.contains(&ExtensionType::Backend) && result.capabilities.backend.is_some() {
        return Err(HostError::Invalid(
            "Extension advertised backend capabilities without declaring Backend".into(),
        ));
    }
    if unique.contains(&ExtensionType::Workspace) && result.capabilities.workspace.is_none() {
        return Err(HostError::Invalid(
            "Extension declared Workspace without workspace capabilities".into(),
        ));
    }
    if !unique.contains(&ExtensionType::Workspace) && result.capabilities.workspace.is_some() {
        return Err(HostError::Invalid(
            "Extension advertised workspace capabilities without declaring Workspace".into(),
        ));
    }
    Ok(Negotiated::new(
        result.protocol_version,
        result.extension,
        result.capabilities,
        legacy_backend,
    ))
}

/// The daemon's full negotiation rules: offered version, identity, the
/// discovery lock on metadata and capabilities, kind and capability
/// consistency, and the schema v1 legacy backend exemption.
#[derive(Debug, Clone)]
pub struct ExpectedChecks {
    expected: ExpectedExtension,
}

impl ExpectedChecks {
    /// Check the guest against what the host expects of it.
    pub fn new(expected: ExpectedExtension) -> Self {
        Self { expected }
    }
}

impl SessionChecks for ExpectedChecks {
    type Error = HostError;

    fn negotiate(
        &mut self,
        offered: &[String],
        result: InitializeResult,
    ) -> Result<Negotiated, HostError> {
        validate_negotiation(self.expected.clone(), offered, result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_extension_sdk::protocol::InitializeResult;
    use morphir_extension_sdk::{
        BackendCapability, ExtensionCapabilities, ExtensionInfo, ExtensionType, FrontendCapability,
    };

    fn info(types: Vec<ExtensionType>) -> ExtensionInfo {
        ExtensionInfo {
            id: "guest".into(),
            name: "Guest".into(),
            version: "1.0.0".into(),
            types,
            ..ExtensionInfo::default()
        }
    }

    fn result(info: ExtensionInfo, capabilities: ExtensionCapabilities) -> InitializeResult {
        InitializeResult {
            protocol_version: "0.1".into(),
            extension: info,
            capabilities,
        }
    }

    fn offered() -> Vec<String> {
        vec!["0.1".into()]
    }

    #[test]
    fn a_discovered_version_must_not_drift() {
        let mut discovered = info(vec![ExtensionType::Backend]);
        discovered.version = "2.0.0".into();
        let backend = ExtensionCapabilities {
            backend: Some(BackendCapability {
                targets: vec!["avro".into()],
                ir_versions: vec!["4".into()],
                generate: true,
            }),
            ..ExtensionCapabilities::default()
        };
        let mut checks = ExpectedChecks::new(ExpectedExtension::discovered(discovered));
        let error = checks
            .negotiate(
                &offered(),
                result(info(vec![ExtensionType::Backend]), backend),
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Extension 'guest' initialization metadata disagreed with discovery: version '1.0.0' was discovered as '2.0.0'"
        );
    }

    #[test]
    fn a_legacy_backend_may_omit_its_capability() {
        let mut checks = ExpectedChecks::new(ExpectedExtension::legacy_discovered(info(vec![
            ExtensionType::Backend,
        ])));
        let negotiated = checks
            .negotiate(
                &offered(),
                result(
                    info(vec![ExtensionType::Backend]),
                    ExtensionCapabilities::default(),
                ),
            )
            .unwrap();
        assert!(negotiated.supports_method(morphir_extension_sdk::protocol::methods::GENERATE));
    }

    /// An installed record cannot carry `multiDocument`, so a guest that
    /// advertises it still agrees with its persisted frontend record, while a
    /// member the record does carry must still match.
    #[test]
    fn a_persisted_frontend_leaves_multi_document_to_the_guest() {
        let persisted = FrontendCapability {
            ir_versions: vec!["3".into()],
            compile: true,
            ..FrontendCapability::default()
        };
        let advertised = FrontendCapability {
            multi_document: true,
            ..persisted.clone()
        };

        assert_eq!(
            persisted_frontend(&persisted, Some(&advertised)),
            advertised
        );

        let drifted = FrontendCapability {
            ir_versions: vec!["4".into()],
            ..advertised.clone()
        };
        let completed = persisted_frontend(&persisted, Some(&drifted));
        assert_ne!(completed, drifted);
        assert_eq!(
            frontend_differences(Some(&drifted), Some(&completed)),
            vec!["irVersions"]
        );
    }

    #[test]
    fn an_exact_capability_lock_names_the_members_that_differ() {
        let locked = ExtensionCapabilities {
            frontend: Some(FrontendCapability {
                compile: true,
                ..FrontendCapability::default()
            }),
            ..ExtensionCapabilities::default()
        };
        let reported = ExtensionCapabilities {
            frontend: Some(FrontendCapability {
                compile: true,
                incremental: true,
                ..FrontendCapability::default()
            }),
            ..ExtensionCapabilities::default()
        };
        let mut checks = ExpectedChecks::new(ExpectedExtension::discovered_with_capabilities(
            info(vec![ExtensionType::Frontend]),
            locked,
        ));
        let error = checks
            .negotiate(
                &offered(),
                result(info(vec![ExtensionType::Frontend]), reported),
            )
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "Extension 'guest' capabilities disagreed with discovery: incremental"
        );
    }
}

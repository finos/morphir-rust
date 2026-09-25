//! Transport-neutral extension provider registration and resolution.
//!
//! The rules and error texts are the daemon's `ExtensionRegistry` rules:
//! resolution filters by language or target and by normalized IR release,
//! installed providers win over built-ins, and more than one match at the
//! best origin is an error.

mod types;

pub use types::{
    CapabilityMetadataScope, GuestSource, InvocationMode, InvocationPolicy, ProviderMetadata,
    ProviderOrigin, Resolved,
};

use crate::HostError;
use morphir_core::format_version::{
    NormalizedFormatVersion, ReleaseTriplet, ScalarValue, SupportTable,
};
use morphir_extension_sdk::{BackendCapability, FrontendCapability};
use std::collections::BTreeMap;
use std::sync::Arc;
use types::Role;

/// In-memory registry of built-in and installed provider sources.
#[derive(Clone, Default)]
pub struct Registry {
    providers: BTreeMap<(ProviderOrigin, String), Arc<Registered>>,
}

/// One source with the IR releases its capabilities advertise, normalized
/// when it registered.
struct Registered {
    source: Arc<dyn GuestSource>,
    frontend_releases: Option<Vec<ReleaseTriplet>>,
    backend_releases: Option<Vec<ReleaseTriplet>>,
}

impl Registered {
    fn origin(&self) -> ProviderOrigin {
        self.source.origin()
    }

    fn id(&self) -> &str {
        &self.source.info().id
    }

    fn frontend(&self) -> Option<(&FrontendCapability, &[ReleaseTriplet])> {
        self.source
            .capabilities()
            .frontend
            .as_ref()
            .zip(self.frontend_releases.as_deref())
    }

    fn backend(&self) -> Option<(&BackendCapability, &[ReleaseTriplet])> {
        self.source
            .capabilities()
            .backend
            .as_ref()
            .zip(self.backend_releases.as_deref())
    }
}

impl Registry {
    /// Create an empty provider registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one provider source.
    ///
    /// Fails when a provider with the same origin and ID is already
    /// registered, or when a frontend or backend capability advertises no IR
    /// version or one that does not normalize to a supported release.
    pub fn register(&mut self, source: Arc<dyn GuestSource>) -> Result<(), HostError> {
        validate_origin_scope_and_modes(source.as_ref())?;
        let info = source.info();
        let origin = source.origin();
        let key = (origin, info.id.clone());
        if self.providers.contains_key(&key) {
            return Err(HostError::Invalid(format!(
                "duplicate {origin:?} provider ID '{}'",
                info.id
            )));
        }

        let capabilities = source.capabilities();
        let frontend_releases = capabilities
            .frontend
            .as_ref()
            .map(|capability| {
                normalize_advertised_releases(&info.id, "frontend", &capability.ir_versions)
            })
            .transpose()?;
        let backend_releases = capabilities
            .backend
            .as_ref()
            .map(|capability| {
                normalize_advertised_releases(&info.id, "backend", &capability.ir_versions)
            })
            .transpose()?;

        self.providers.insert(
            key,
            Arc::new(Registered {
                source,
                frontend_releases,
                backend_releases,
            }),
        );
        Ok(())
    }

    /// Resolve an enabled frontend for an exact language and normalized IR release.
    pub fn resolve_frontend(
        &self,
        language_id: &str,
        ir_version: &str,
        policy: InvocationPolicy,
    ) -> Result<Resolved, HostError> {
        let requested = normalize_requested_ir_version(ir_version)?;
        let matching: Vec<_> = self
            .providers
            .values()
            .filter(|provider| {
                provider.frontend().is_some_and(|(capability, releases)| {
                    capability.compile
                        && capability
                            .languages
                            .iter()
                            .any(|language| language.id == language_id)
                        && releases.contains(&requested)
                })
            })
            .cloned()
            .collect();
        let provider = select_provider(
            matching,
            "frontend.compile",
            &format!("language '{language_id}'"),
            requested,
            || self.frontend_candidates(),
        )?;
        Ok(resolved(&provider, Role::Frontend, policy))
    }

    /// Resolve an enabled backend for an exact target and normalized IR release.
    pub fn resolve_backend(
        &self,
        target: &str,
        ir_version: &str,
        policy: InvocationPolicy,
    ) -> Result<Resolved, HostError> {
        let requested = normalize_requested_ir_version(ir_version)?;
        let matching: Vec<_> = self
            .providers
            .values()
            .filter(|provider| {
                provider.backend().is_some_and(|(capability, releases)| {
                    capability.generate
                        && capability
                            .targets
                            .iter()
                            .any(|candidate| candidate == target)
                        && releases.contains(&requested)
                })
            })
            .cloned()
            .collect();
        let provider = select_provider(
            matching,
            "backend.generate",
            &format!("target '{target}'"),
            requested,
            || self.backend_candidates(),
        )?;
        Ok(resolved(&provider, Role::Backend, policy))
    }

    /// List immutable provider metadata in origin-then-ID order.
    pub fn providers(&self) -> Vec<ProviderMetadata> {
        self.providers
            .values()
            .map(|provider| ProviderMetadata {
                source: Arc::clone(&provider.source),
            })
            .collect()
    }

    fn frontend_candidates(&self) -> String {
        let candidates = self.providers.values().filter_map(|provider| {
            provider.frontend().map(|(capability, releases)| {
                let mut languages: Vec<_> = capability
                    .languages
                    .iter()
                    .map(|language| language.id.as_str())
                    .collect();
                languages.sort_unstable();
                format_candidate(
                    provider,
                    &format!("languages={languages:?}"),
                    releases,
                    &format!("compile={}", capability.compile),
                )
            })
        });
        join_candidates(candidates)
    }

    fn backend_candidates(&self) -> String {
        let candidates = self.providers.values().filter_map(|provider| {
            provider.backend().map(|(capability, releases)| {
                let mut targets: Vec<_> = capability.targets.iter().map(String::as_str).collect();
                targets.sort_unstable();
                format_candidate(
                    provider,
                    &format!("targets={targets:?}"),
                    releases,
                    &format!("generate={}", capability.generate),
                )
            })
        });
        join_candidates(candidates)
    }
}

/// Reject a source whose origin disagrees with the capability metadata scope
/// or invocation modes the daemon's mapping expects of that origin.
///
/// A `Builtin` source must report `Complete` scope, `NativeDirect` under
/// `PreferDirect`, and `NativeMep` under `ProtocolOnly`. An `Installed`
/// source must report `PersistedFrontendBackend` scope and either
/// `ProcessMep` or `WasmMep`, the same mode under both policies.
fn validate_origin_scope_and_modes(source: &dyn GuestSource) -> Result<(), HostError> {
    let origin = source.origin();
    let scope = source.capability_metadata_scope();
    let prefer_direct = source.invocation_mode(InvocationPolicy::PreferDirect);
    let protocol_only = source.invocation_mode(InvocationPolicy::ProtocolOnly);
    let agrees = match origin {
        ProviderOrigin::Builtin => {
            scope == CapabilityMetadataScope::Complete
                && prefer_direct == InvocationMode::NativeDirect
                && protocol_only == InvocationMode::NativeMep
        }
        ProviderOrigin::Installed => {
            scope == CapabilityMetadataScope::PersistedFrontendBackend
                && prefer_direct == protocol_only
                && matches!(
                    prefer_direct,
                    InvocationMode::ProcessMep | InvocationMode::WasmMep
                )
        }
    };
    if agrees {
        Ok(())
    } else {
        Err(HostError::Invalid(format!(
            "provider '{}' reports {scope:?}, {prefer_direct:?} under PreferDirect and {protocol_only:?} under ProtocolOnly, which do not match its {origin:?} origin",
            source.info().id
        )))
    }
}

fn resolved(provider: &Registered, role: Role, policy: InvocationPolicy) -> Resolved {
    Resolved {
        source: Arc::clone(&provider.source),
        invocation_mode: provider.source.invocation_mode(policy),
        role,
    }
}

fn select_provider(
    matching: Vec<Arc<Registered>>,
    capability: &str,
    selector: &str,
    requested: ReleaseTriplet,
    candidates: impl FnOnce() -> String,
) -> Result<Arc<Registered>, HostError> {
    let Some(best_origin) = matching.iter().map(|provider| provider.origin()).max() else {
        return Err(HostError::Invalid(format!(
            "no provider supports {capability} for {selector} at IR {requested}; relevant providers: {}",
            candidates()
        )));
    };
    let mut best: Vec<_> = matching
        .into_iter()
        .filter(|provider| provider.origin() == best_origin)
        .collect();
    best.sort_by(|left, right| left.id().cmp(right.id()));
    if best.len() > 1 {
        let ids: Vec<_> = best.iter().map(|provider| provider.id()).collect();
        return Err(HostError::Invalid(format!(
            "ambiguous {capability} provider for {selector} at IR {requested} from {best_origin:?}: {ids:?}"
        )));
    }
    Ok(best.remove(0))
}

fn normalize_advertised_releases(
    provider_id: &str,
    capability: &str,
    values: &[String],
) -> Result<Vec<ReleaseTriplet>, HostError> {
    if values.is_empty() {
        return Err(HostError::Invalid(format!(
            "provider '{provider_id}' must advertise at least one {capability} IR version"
        )));
    }
    values
        .iter()
        .map(|value| {
            normalize_ir_version(value).map_err(|failure| {
                HostError::Invalid(format!(
                    "provider '{provider_id}' advertised invalid {capability} IR version '{value}': {failure}"
                ))
            })
        })
        .collect()
}

fn normalize_requested_ir_version(value: &str) -> Result<ReleaseTriplet, HostError> {
    normalize_ir_version(value).map_err(|failure| match failure {
        VersionFailure::Malformed(detail) => HostError::Invalid(format!(
            "requested IR version '{value}' is malformed: {detail}"
        )),
        VersionFailure::Unsupported(detail) => HostError::Invalid(format!(
            "requested IR version '{value}' is unsupported: {detail}"
        )),
    })
}

#[derive(Debug)]
enum VersionFailure {
    Malformed(String),
    Unsupported(String),
}

impl std::fmt::Display for VersionFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(detail) | Self::Unsupported(detail) => formatter.write_str(detail),
        }
    }
}

fn normalize_ir_version(value: &str) -> Result<ReleaseTriplet, VersionFailure> {
    let scalar = if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        ScalarValue::Integer(
            value
                .parse::<u64>()
                .map_err(|error| VersionFailure::Malformed(error.to_string()))?,
        )
    } else {
        ScalarValue::String(value.to_owned())
    };
    let support = SupportTable::reference();
    let normalized = NormalizedFormatVersion::from_scalar(&scalar, &support)
        .map_err(|error| VersionFailure::Malformed(error.to_string()))?;
    if !normalized.is_supported() {
        let detail = support
            .unsupported_diagnostic(&normalized.release, normalized.compatibility)
            .map(|diagnostic| diagnostic.to_string())
            .unwrap_or_else(|| normalized.release.to_string());
        return Err(VersionFailure::Unsupported(detail));
    }
    Ok(normalized.release)
}

fn format_candidate(
    provider: &Registered,
    selector: &str,
    releases: &[ReleaseTriplet],
    enabled: &str,
) -> String {
    let mut versions: Vec<_> = releases.iter().map(ToString::to_string).collect();
    versions.sort();
    format!(
        "{} [{:?}] {selector} irVersions={versions:?} {enabled}",
        provider.id(),
        provider.origin()
    )
}

fn join_candidates(candidates: impl Iterator<Item = String>) -> String {
    let candidates: Vec<_> = candidates.collect();
    if candidates.is_empty() {
        "none".to_owned()
    } else {
        candidates.join("; ")
    }
}

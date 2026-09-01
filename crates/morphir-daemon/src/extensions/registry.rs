//! Transport-neutral extension provider registration and resolution.

mod types;

pub use types::{
    CapabilityMetadataScope, InvocationMode, InvocationPolicy, ProviderMetadata, ProviderOrigin,
    ResolvedBackend, ResolvedFrontend,
};

use crate::{DaemonError, Result};
use morphir_core::format_version::{
    NormalizedFormatVersion, ReleaseTriplet, ScalarValue, SupportTable,
};
use morphir_distribution::InstalledExtensionSnapshot;
use morphir_extension_sdk::{ExtensionCapabilities, ExtensionInfo, NativeExtension};
use std::collections::BTreeMap;
use std::sync::Arc;
use types::{ProviderRuntime, RegisteredBackend, RegisteredFrontend, RegisteredProvider};

/// In-memory registry of immutable built-in and installed provider snapshots.
#[derive(Clone, Default)]
pub struct ExtensionRegistry {
    providers: BTreeMap<(ProviderOrigin, String), Arc<RegisteredProvider>>,
}

impl ExtensionRegistry {
    /// Create an empty provider registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one in-process provider using its native metadata snapshots.
    pub fn register_builtin(&mut self, extension: NativeExtension) -> Result<()> {
        let info = extension.info().clone();
        let capabilities = extension.capabilities().clone();
        self.register_provider(
            info,
            capabilities,
            ProviderOrigin::Builtin,
            ProviderRuntime::Native(extension),
        )
    }

    /// Register one atomically validated installed provider snapshot.
    pub fn register_installed(&mut self, snapshot: InstalledExtensionSnapshot) -> Result<()> {
        let info = snapshot.installed().extension_info();
        let capabilities = snapshot.installed().extension_capabilities();
        self.register_provider(
            info,
            capabilities,
            ProviderOrigin::Installed,
            ProviderRuntime::Installed(snapshot),
        )
    }

    fn register_provider(
        &mut self,
        info: ExtensionInfo,
        capabilities: ExtensionCapabilities,
        origin: ProviderOrigin,
        runtime: ProviderRuntime,
    ) -> Result<()> {
        let key = (origin, info.id.clone());
        if self.providers.contains_key(&key) {
            return Err(DaemonError::Extension(format!(
                "duplicate {origin:?} provider ID '{}'",
                info.id
            )));
        }

        let frontend = capabilities
            .frontend
            .clone()
            .map(|capability| {
                normalize_advertised_releases(&info.id, "frontend", &capability.ir_versions).map(
                    |releases| RegisteredFrontend {
                        capability,
                        releases,
                    },
                )
            })
            .transpose()?;
        let backend = capabilities
            .backend
            .clone()
            .map(|capability| {
                normalize_advertised_releases(&info.id, "backend", &capability.ir_versions).map(
                    |releases| RegisteredBackend {
                        capability,
                        releases,
                    },
                )
            })
            .transpose()?;

        self.providers.insert(
            key,
            Arc::new(RegisteredProvider {
                info,
                capabilities,
                origin,
                capability_metadata_scope: match origin {
                    ProviderOrigin::Builtin => CapabilityMetadataScope::Complete,
                    ProviderOrigin::Installed => CapabilityMetadataScope::PersistedFrontendBackend,
                },
                runtime,
                frontend,
                backend,
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
    ) -> Result<ResolvedFrontend> {
        let requested = normalize_requested_ir_version(ir_version)?;
        let matching: Vec<_> = self
            .providers
            .values()
            .filter(|provider| {
                provider.frontend.as_ref().is_some_and(|frontend| {
                    frontend.capability.compile
                        && frontend
                            .capability
                            .languages
                            .iter()
                            .any(|language| language.id == language_id)
                        && frontend.releases.contains(&requested)
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
        provider
            .frontend
            .as_ref()
            .expect("selected frontend provider retains frontend metadata");
        Ok(ResolvedFrontend {
            invocation_mode: provider.runtime.invocation_mode(policy),
            provider,
        })
    }

    /// Resolve an enabled backend for an exact target and normalized IR release.
    pub fn resolve_backend(
        &self,
        target: &str,
        ir_version: &str,
        policy: InvocationPolicy,
    ) -> Result<ResolvedBackend> {
        let requested = normalize_requested_ir_version(ir_version)?;
        let matching: Vec<_> = self
            .providers
            .values()
            .filter(|provider| {
                provider.backend.as_ref().is_some_and(|backend| {
                    backend.capability.generate
                        && backend
                            .capability
                            .targets
                            .iter()
                            .any(|candidate| candidate == target)
                        && backend.releases.contains(&requested)
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
        provider
            .backend
            .as_ref()
            .expect("selected backend provider retains backend metadata");
        Ok(ResolvedBackend {
            invocation_mode: provider.runtime.invocation_mode(policy),
            provider,
        })
    }

    /// List immutable provider metadata in origin-then-ID order.
    pub fn providers(&self) -> Vec<ProviderMetadata> {
        self.providers
            .values()
            .map(|provider| ProviderMetadata {
                provider: Arc::clone(provider),
            })
            .collect()
    }

    fn frontend_candidates(&self) -> String {
        let candidates = self.providers.values().filter_map(|provider| {
            provider.frontend.as_ref().map(|frontend| {
                let mut languages: Vec<_> = frontend
                    .capability
                    .languages
                    .iter()
                    .map(|language| language.id.as_str())
                    .collect();
                languages.sort_unstable();
                format_candidate(
                    provider,
                    &format!("languages={languages:?}"),
                    &frontend.releases,
                    &format!("compile={}", frontend.capability.compile),
                )
            })
        });
        join_candidates(candidates)
    }

    fn backend_candidates(&self) -> String {
        let candidates = self.providers.values().filter_map(|provider| {
            provider.backend.as_ref().map(|backend| {
                let mut targets: Vec<_> = backend
                    .capability
                    .targets
                    .iter()
                    .map(String::as_str)
                    .collect();
                targets.sort_unstable();
                format_candidate(
                    provider,
                    &format!("targets={targets:?}"),
                    &backend.releases,
                    &format!("generate={}", backend.capability.generate),
                )
            })
        });
        join_candidates(candidates)
    }
}

fn select_provider(
    matching: Vec<Arc<RegisteredProvider>>,
    capability: &str,
    selector: &str,
    requested: ReleaseTriplet,
    candidates: impl FnOnce() -> String,
) -> Result<Arc<RegisteredProvider>> {
    let Some(best_origin) = matching.iter().map(|provider| provider.origin).max() else {
        return Err(DaemonError::Extension(format!(
            "no provider supports {capability} for {selector} at IR {requested}; relevant providers: {}",
            candidates()
        )));
    };
    let mut best: Vec<_> = matching
        .into_iter()
        .filter(|provider| provider.origin == best_origin)
        .collect();
    best.sort_by(|left, right| left.info.id.cmp(&right.info.id));
    if best.len() > 1 {
        let ids: Vec<_> = best
            .iter()
            .map(|provider| provider.info.id.as_str())
            .collect();
        return Err(DaemonError::Extension(format!(
            "ambiguous {capability} provider for {selector} at IR {requested} from {best_origin:?}: {ids:?}"
        )));
    }
    Ok(best.remove(0))
}

fn normalize_advertised_releases(
    provider_id: &str,
    capability: &str,
    values: &[String],
) -> Result<Vec<ReleaseTriplet>> {
    if values.is_empty() {
        return Err(DaemonError::Extension(format!(
            "provider '{provider_id}' must advertise at least one {capability} IR version"
        )));
    }
    values
        .iter()
        .map(|value| {
            normalize_ir_version(value).map_err(|failure| {
                DaemonError::Extension(format!(
                    "provider '{provider_id}' advertised invalid {capability} IR version '{value}': {failure}"
                ))
            })
        })
        .collect()
}

fn normalize_requested_ir_version(value: &str) -> Result<ReleaseTriplet> {
    normalize_ir_version(value).map_err(|failure| match failure {
        VersionFailure::Malformed(detail) => DaemonError::Extension(format!(
            "requested IR version '{value}' is malformed: {detail}"
        )),
        VersionFailure::Unsupported(detail) => DaemonError::Extension(format!(
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

fn normalize_ir_version(value: &str) -> std::result::Result<ReleaseTriplet, VersionFailure> {
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
    provider: &RegisteredProvider,
    selector: &str,
    releases: &[ReleaseTriplet],
    enabled: &str,
) -> String {
    let mut versions: Vec<_> = releases.iter().map(ToString::to_string).collect();
    versions.sort();
    format!(
        "{} [{:?}] {selector} irVersions={versions:?} {enabled}",
        provider.info.id, provider.origin
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

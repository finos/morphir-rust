//! Provider resolution rules, ported from the daemon's `provider_registry`
//! tests. Every source here is a `FakeSource`: the rules need only metadata.
//! The tests that need a real built-in live in
//! `morphir-host-native/tests/native_source.rs`.

use morphir_extension_sdk::protocol::{ExtensionResponse, PeerInfo, PeerKind};
use morphir_extension_sdk::{
    BackendCapability, ExtensionCapabilities, ExtensionInfo, ExtensionType, FrontendCapability,
    LanguageCapability, WorkspaceCapability,
};
use morphir_host::testing::{FakeSource, frontend_initialize_result};
use morphir_host::{
    CapabilityMetadataScope, GuestSource, HostConfig, InvocationMode, InvocationPolicy,
    ProviderOrigin, Registry, Session,
};
use std::path::Path;
use std::sync::Arc;

fn info(id: &str, name: String, version: &str, types: Vec<ExtensionType>) -> ExtensionInfo {
    ExtensionInfo {
        id: id.into(),
        name,
        version: version.into(),
        types,
        ..ExtensionInfo::default()
    }
}

fn frontend(language: &str, ir_version: &str, compile: bool) -> FrontendCapability {
    FrontendCapability {
        languages: vec![LanguageCapability {
            id: language.into(),
            file_extensions: vec![format!(".{language}")],
        }],
        ir_versions: vec![ir_version.into()],
        compile,
        incremental: false,
        fragments: false,
        multi_document: false,
    }
}

fn backend(target: &str, ir_version: &str, generate: bool) -> BackendCapability {
    BackendCapability {
        targets: vec![target.into()],
        ir_versions: vec![ir_version.into()],
        generate,
    }
}

/// A built-in-like provider with one frontend language and one backend target.
fn builtin(id: &str, language: &str, target: &str, ir_version: &str) -> Arc<FakeSource> {
    Arc::new(FakeSource::builtin(
        info(id, format!("Test provider {id}"), "1.0.0", Vec::new()),
        ExtensionCapabilities {
            frontend: Some(frontend(language, ir_version, true)),
            backend: Some(backend(target, ir_version, true)),
            ..ExtensionCapabilities::default()
        },
    ))
}

fn builtin_alpha() -> Arc<FakeSource> {
    builtin("builtin-alpha", "gleam", "json", "4.0.0")
}

fn builtin_zulu() -> Arc<FakeSource> {
    builtin("builtin-zulu", "gleam", "json", "4.0.0")
}

fn alias_four() -> Arc<FakeSource> {
    builtin("alias-four", "alias-lang", "alias-target", "4")
}

fn exact_four() -> Arc<FakeSource> {
    builtin("exact-four", "exact-lang", "exact-target", "4.0.0")
}

fn same_id() -> Arc<FakeSource> {
    builtin("same-provider", "gleam", "json", "4")
}

/// An installed-like provider whose runtime fixes its invocation mode.
fn installed(
    id: &str,
    mode: InvocationMode,
    frontend_spec: Option<(&str, &str, bool)>,
    backend_spec: Option<(&str, &str, bool)>,
) -> Arc<FakeSource> {
    let types = [
        frontend_spec.map(|_| ExtensionType::Frontend),
        backend_spec.map(|_| ExtensionType::Backend),
    ]
    .into_iter()
    .flatten()
    .collect();
    Arc::new(FakeSource::installed(
        info(id, format!("Installed {id}"), "2.0.0", types),
        ExtensionCapabilities {
            frontend: frontend_spec
                .map(|(language, ir_version, compile)| frontend(language, ir_version, compile)),
            backend: backend_spec
                .map(|(target, ir_version, generate)| backend(target, ir_version, generate)),
            ..ExtensionCapabilities::default()
        },
        mode,
    ))
}

fn process_provider(id: &str, language: &str, target: &str) -> Arc<FakeSource> {
    installed(
        id,
        InvocationMode::ProcessMep,
        Some((language, "4", true)),
        Some((target, "4.0.0", true)),
    )
}

#[test]
fn resolves_frontend_and_backend_by_capability_instead_of_provider_id() {
    let mut registry = Registry::new();
    registry.register(builtin_alpha()).unwrap();

    let frontend = registry
        .resolve_frontend("gleam", "4", InvocationPolicy::PreferDirect)
        .unwrap();
    let backend = registry
        .resolve_backend("json", "4.0.0", InvocationPolicy::PreferDirect)
        .unwrap();

    assert_eq!(frontend.info().id, "builtin-alpha");
    assert_eq!(frontend.frontend().unwrap().languages[0].id, "gleam");
    assert!(frontend.backend().is_none());
    assert_eq!(frontend.origin(), ProviderOrigin::Builtin);
    assert_eq!(frontend.invocation_mode(), InvocationMode::NativeDirect);
    assert_eq!(backend.info().id, "builtin-alpha");
    assert_eq!(backend.backend().unwrap().targets, ["json"]);
    assert!(backend.frontend().is_none());
    assert_eq!(backend.origin(), ProviderOrigin::Builtin);
}

#[test]
fn major_alias_and_exact_baseline_release_match_in_both_directions() {
    let mut registry = Registry::new();
    registry.register(alias_four()).unwrap();
    registry.register(exact_four()).unwrap();

    assert_eq!(
        registry
            .resolve_frontend("exact-lang", "4", InvocationPolicy::PreferDirect)
            .unwrap()
            .info()
            .id,
        "exact-four"
    );
    assert_eq!(
        registry
            .resolve_backend("alias-target", "4.0.0", InvocationPolicy::PreferDirect)
            .unwrap()
            .info()
            .id,
        "alias-four"
    );
    assert!(
        registry
            .resolve_frontend("exact-lang", "4.0.0", InvocationPolicy::PreferDirect)
            .is_ok()
    );
}

#[test]
fn registration_rejects_malformed_whitespace_and_unsupported_advertised_ir_versions() {
    for (provider, expected) in [
        (builtin("whitespace-ir", "space", "space", " 4"), " 4"),
        (
            builtin("malformed-ir", "malformed", "malformed", "four"),
            "four",
        ),
        (builtin("unsupported-ir", "future", "future", "5"), "5"),
    ] {
        let mut registry = Registry::new();
        let error = registry.register(provider).unwrap_err().to_string();
        assert!(error.contains("advertised"), "{error}");
        assert!(error.contains(expected), "{error}");
    }

    let mut capabilities = exact_four().capabilities().clone();
    capabilities.frontend.as_mut().unwrap().ir_versions.clear();
    capabilities.backend.as_mut().unwrap().ir_versions.clear();
    let empty_versions = FakeSource::builtin(
        info(
            "empty-versions",
            "Empty versions".into(),
            "1.0.0",
            Vec::new(),
        ),
        capabilities,
    );
    let mut registry = Registry::new();
    let error = registry
        .register(Arc::new(empty_versions))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("must advertise at least one frontend IR version"),
        "{error}"
    );
}

#[test]
fn register_builtin_admits_a_workspace_only_provider_and_retains_its_capabilities() {
    let workspace = WorkspaceCapability {
        protocol_versions: vec![morphir_workspace::workspace_discovery_protocol()],
        discover: true,
    };
    let mut registry = Registry::new();
    registry
        .register(Arc::new(FakeSource::builtin(
            info(
                "workspace-only",
                "Workspace only".into(),
                "1.0.0",
                Vec::new(),
            ),
            ExtensionCapabilities {
                workspace: Some(workspace.clone()),
                ..ExtensionCapabilities::default()
            },
        )))
        .unwrap();

    let listed = registry.providers();
    let workspace_only = listed
        .iter()
        .find(|provider| provider.info().id == "workspace-only")
        .expect("workspace-only provider should be listed after registration");
    assert_eq!(workspace_only.capabilities().workspace, Some(workspace));
    assert!(workspace_only.capabilities().frontend.is_none());
    assert!(workspace_only.capabilities().backend.is_none());
}

#[test]
fn malformed_and_unsupported_requested_ir_versions_are_rejected_before_resolution() {
    let mut registry = Registry::new();
    registry.register(exact_four()).unwrap();

    let malformed = registry
        .resolve_frontend("exact-lang", " 4", InvocationPolicy::PreferDirect)
        .unwrap_err()
        .to_string();
    let unsupported = registry
        .resolve_frontend("exact-lang", "5", InvocationPolicy::PreferDirect)
        .unwrap_err()
        .to_string();

    assert!(malformed.contains("requested IR version ' 4' is malformed"));
    assert!(unsupported.contains("requested IR version '5' is unsupported"));
}

#[test]
fn matching_installed_provider_overrides_builtin_for_each_typed_capability() {
    let mut registry = Registry::new();
    registry.register(builtin_alpha()).unwrap();
    registry
        .register(process_provider("installed-choice", "gleam", "json"))
        .unwrap();

    let frontend = registry
        .resolve_frontend("gleam", "4", InvocationPolicy::PreferDirect)
        .unwrap();
    let backend = registry
        .resolve_backend("json", "4", InvocationPolicy::PreferDirect)
        .unwrap();

    assert_eq!(frontend.info().id, "installed-choice");
    assert_eq!(frontend.origin(), ProviderOrigin::Installed);
    assert_eq!(
        frontend.capability_metadata_scope(),
        CapabilityMetadataScope::PersistedFrontendBackend
    );
    assert_eq!(frontend.invocation_mode(), InvocationMode::ProcessMep);
    assert_eq!(backend.info().id, "installed-choice");
    assert_eq!(backend.origin(), ProviderOrigin::Installed);
}

#[test]
fn nonmatching_installed_provider_does_not_suppress_matching_builtin() {
    let mut registry = Registry::new();
    registry.register(builtin_alpha()).unwrap();
    registry
        .register(installed(
            "installed-other-version",
            InvocationMode::ProcessMep,
            Some(("gleam", "3", true)),
            Some(("json", "3.0.0", true)),
        ))
        .unwrap();

    assert_eq!(
        registry
            .resolve_frontend("gleam", "4", InvocationPolicy::PreferDirect)
            .unwrap()
            .info()
            .id,
        "builtin-alpha"
    );
    assert_eq!(
        registry
            .resolve_backend("json", "4", InvocationPolicy::PreferDirect)
            .unwrap()
            .info()
            .id,
        "builtin-alpha"
    );
}

#[test]
fn ambiguity_at_the_best_origin_reports_sorted_provider_ids() {
    let mut registry = Registry::new();
    registry.register(builtin_zulu()).unwrap();
    registry.register(builtin_alpha()).unwrap();

    let error = registry
        .resolve_frontend("gleam", "4", InvocationPolicy::PreferDirect)
        .unwrap_err()
        .to_string();

    assert!(error.contains("ambiguous"), "{error}");
    assert!(
        error.find("builtin-alpha").unwrap() < error.find("builtin-zulu").unwrap(),
        "{error}"
    );
}

#[test]
fn duplicate_ids_are_rejected_within_an_origin_but_allowed_across_origins() {
    let mut registry = Registry::new();
    registry.register(same_id()).unwrap();
    let error = registry.register(same_id()).unwrap_err().to_string();
    assert!(error.contains("duplicate Builtin provider ID 'same-provider'"));

    registry
        .register(process_provider("same-provider", "gleam", "json"))
        .unwrap();
    let duplicate_installed = process_provider("same-provider", "gleam", "json");
    let error = registry
        .register(duplicate_installed)
        .unwrap_err()
        .to_string();
    assert!(error.contains("duplicate Installed provider ID 'same-provider'"));

    assert_eq!(
        registry
            .resolve_frontend("gleam", "4", InvocationPolicy::PreferDirect)
            .unwrap()
            .origin(),
        ProviderOrigin::Installed
    );
}

#[test]
fn installed_runtime_selects_process_or_wasm_protocol_mode_regardless_of_policy() {
    let mut registry = Registry::new();
    registry
        .register(process_provider(
            "process-provider",
            "process-lang",
            "process-target",
        ))
        .unwrap();
    registry
        .register(installed(
            "wasm-provider",
            InvocationMode::WasmMep,
            Some(("wasm-lang", "4", true)),
            Some(("wasm-target", "4", true)),
        ))
        .unwrap();

    assert_eq!(
        registry
            .resolve_frontend("process-lang", "4", InvocationPolicy::PreferDirect)
            .unwrap()
            .invocation_mode(),
        InvocationMode::ProcessMep
    );
    assert_eq!(
        registry
            .resolve_backend("wasm-target", "4", InvocationPolicy::ProtocolOnly)
            .unwrap()
            .invocation_mode(),
        InvocationMode::WasmMep
    );
}

#[test]
fn disabled_typed_operations_are_not_resolvable() {
    let mut registry = Registry::new();
    registry
        .register(installed(
            "disabled-provider",
            InvocationMode::ProcessMep,
            Some(("disabled-lang", "4", false)),
            Some(("disabled-target", "4", false)),
        ))
        .unwrap();

    assert!(
        registry
            .resolve_frontend("disabled-lang", "4", InvocationPolicy::PreferDirect)
            .is_err()
    );
    assert!(
        registry
            .resolve_backend("disabled-target", "4", InvocationPolicy::PreferDirect)
            .is_err()
    );
}

#[test]
fn no_match_diagnostic_reports_capability_candidates_origins_and_version_context() {
    let mut registry = Registry::new();
    registry.register(exact_four()).unwrap();
    registry
        .register(process_provider("installed-v4", "gleam", "json"))
        .unwrap();

    let version_error = registry
        .resolve_frontend("gleam", "3", InvocationPolicy::PreferDirect)
        .unwrap_err()
        .to_string();
    assert!(
        version_error.contains("frontend.compile"),
        "{version_error}"
    );
    assert!(
        version_error.contains("language 'gleam'"),
        "{version_error}"
    );
    assert!(version_error.contains("IR 3.0.0"), "{version_error}");
    assert!(
        version_error.contains("installed-v4 [Installed]"),
        "{version_error}"
    );
    assert!(version_error.contains("4.0.0"), "{version_error}");

    let language_error = registry
        .resolve_frontend("missing", "4", InvocationPolicy::PreferDirect)
        .unwrap_err()
        .to_string();
    assert!(
        language_error.contains("exact-four [Builtin]"),
        "{language_error}"
    );
    assert!(
        language_error.contains("installed-v4 [Installed]"),
        "{language_error}"
    );
}

#[test]
fn provider_listing_is_stable_and_carries_scoped_metadata_and_default_mode() {
    let mut registry = Registry::new();
    registry
        .register(process_provider("installed-zulu", "gleam", "json"))
        .unwrap();
    registry.register(exact_four()).unwrap();
    registry.register(alias_four()).unwrap();

    let providers = registry.providers();
    let ordered: Vec<_> = providers
        .iter()
        .map(|provider| {
            (
                provider.origin(),
                provider.info().id.as_str(),
                provider.preferred_invocation_mode(),
            )
        })
        .collect();

    assert_eq!(
        ordered,
        vec![
            (
                ProviderOrigin::Builtin,
                "alias-four",
                InvocationMode::NativeDirect
            ),
            (
                ProviderOrigin::Builtin,
                "exact-four",
                InvocationMode::NativeDirect
            ),
            (
                ProviderOrigin::Installed,
                "installed-zulu",
                InvocationMode::ProcessMep
            ),
        ]
    );
    assert!(providers[0].capabilities().frontend.is_some());
    assert!(providers[0].capabilities().backend.is_some());
}

#[test]
fn resolutions_and_listing_share_one_immutable_provider_entry() {
    let mut registry = Registry::new();
    registry.register(exact_four()).unwrap();

    let frontend = registry
        .resolve_frontend("exact-lang", "4", InvocationPolicy::PreferDirect)
        .unwrap();
    let repeated_frontend = registry
        .resolve_frontend("exact-lang", "4", InvocationPolicy::ProtocolOnly)
        .unwrap();
    let backend = registry
        .resolve_backend("exact-target", "4", InvocationPolicy::PreferDirect)
        .unwrap();
    let listed = registry.providers().pop().unwrap();

    assert!(std::ptr::eq(frontend.info(), backend.info()));
    assert!(std::ptr::eq(
        frontend.frontend().unwrap(),
        repeated_frontend.frontend().unwrap()
    ));
    assert!(std::ptr::eq(frontend.info(), listed.info()));
    assert!(std::ptr::eq(
        frontend.capabilities(),
        backend.capabilities()
    ));
    assert!(std::ptr::eq(frontend.capabilities(), listed.capabilities()));
}

#[test]
fn capability_metadata_scope_distinguishes_complete_and_persisted_views() {
    let mut registry = Registry::new();
    registry.register(exact_four()).unwrap();
    registry
        .register(process_provider(
            "installed-scope",
            "installed-lang",
            "installed-target",
        ))
        .unwrap();

    let builtin = registry
        .resolve_frontend("exact-lang", "4", InvocationPolicy::PreferDirect)
        .unwrap();
    let installed = registry
        .resolve_backend("installed-target", "4", InvocationPolicy::PreferDirect)
        .unwrap();
    assert_eq!(
        builtin.capability_metadata_scope(),
        CapabilityMetadataScope::Complete
    );
    assert_eq!(
        installed.capability_metadata_scope(),
        CapabilityMetadataScope::PersistedFrontendBackend
    );

    let listed = registry.providers();
    assert_eq!(
        listed
            .iter()
            .find(|provider| provider.info().id == "exact-four")
            .unwrap()
            .capability_metadata_scope(),
        CapabilityMetadataScope::Complete
    );
    assert_eq!(
        listed
            .iter()
            .find(|provider| provider.info().id == "installed-scope")
            .unwrap()
            .capability_metadata_scope(),
        CapabilityMetadataScope::PersistedFrontendBackend
    );
}

#[test]
fn installed_frontend_claims_resolve_for_multiple_documents() {
    for mode in [InvocationMode::ProcessMep, InvocationMode::WasmMep] {
        let source = installed("multi-document", mode, Some(("elm", "3", true)), None);
        let mut capabilities = source.capabilities().clone();
        let claimed = capabilities.frontend.as_mut().unwrap();
        claimed.multi_document = true;
        claimed.fragments = true;
        claimed.incremental = true;
        let source = FakeSource::installed(source.info().clone(), capabilities, mode);
        let mut registry = Registry::new();
        registry.register(Arc::new(source)).unwrap();
        let frontend = registry
            .resolve_frontend("elm", "3", InvocationPolicy::ProtocolOnly)
            .unwrap();
        assert_eq!(frontend.origin(), ProviderOrigin::Installed);
        assert!(
            frontend.frontend().unwrap().multi_document,
            "the resolved provider must permit multiple source documents"
        );
        assert!(frontend.frontend().unwrap().fragments);
        assert!(frontend.frontend().unwrap().incremental);
        assert!(
            registry.providers()[0]
                .capabilities()
                .frontend
                .as_ref()
                .unwrap()
                .multi_document
        );
    }
}

#[test]
fn an_installed_provider_reports_workspace_discovery_from_its_persisted_types() {
    let mut registry = Registry::new();
    registry
        .register(process_provider("installed-plain", "plain-lang", "plain"))
        .unwrap();
    let mut discovering = installed(
        "installed-discovering",
        InvocationMode::ProcessMep,
        Some(("discovering-lang", "4", true)),
        None,
    )
    .info()
    .clone();
    discovering.types.push(ExtensionType::Workspace);
    registry
        .register(Arc::new(FakeSource::installed(
            discovering,
            ExtensionCapabilities {
                frontend: Some(frontend("discovering-lang", "4", true)),
                ..ExtensionCapabilities::default()
            },
            InvocationMode::ProcessMep,
        )))
        .unwrap();

    assert!(
        !registry
            .resolve_frontend("plain-lang", "4", InvocationPolicy::PreferDirect)
            .unwrap()
            .supports_workspace_discovery()
    );
    assert!(
        registry
            .resolve_frontend("discovering-lang", "4", InvocationPolicy::PreferDirect)
            .unwrap()
            .supports_workspace_discovery()
    );
}

#[test]
fn a_resolved_provider_has_a_fingerprint_of_its_identity_origin_and_mode() {
    let mut registry = Registry::new();
    registry.register(exact_four()).unwrap();
    registry
        .register(process_provider("installed-choice", "gleam", "json"))
        .unwrap();

    assert_eq!(
        registry
            .resolve_frontend("exact-lang", "4", InvocationPolicy::PreferDirect)
            .unwrap()
            .fingerprint(),
        "exact-four@1.0.0:Builtin:NativeDirect"
    );
    assert_eq!(
        registry
            .resolve_backend("exact-target", "4", InvocationPolicy::ProtocolOnly)
            .unwrap()
            .fingerprint(),
        "exact-four@1.0.0:Builtin:NativeMep"
    );
    assert_eq!(
        registry
            .resolve_backend("json", "4", InvocationPolicy::PreferDirect)
            .unwrap()
            .fingerprint(),
        "installed-choice@2.0.0:Installed:ProcessMep"
    );
}

#[tokio::test]
async fn a_resolved_provider_connects_through_its_source() {
    let source = FakeSource::installed(
        info(
            "guest",
            "Installed guest".into(),
            "2.0.0",
            vec![ExtensionType::Frontend],
        ),
        ExtensionCapabilities {
            frontend: Some(frontend("guest-lang", "4", true)),
            ..ExtensionCapabilities::default()
        },
        InvocationMode::ProcessMep,
    )
    .respond(ExtensionResponse::success(1, frontend_initialize_result("guest")).unwrap())
    .respond(ExtensionResponse::success(2, serde_json::json!({})).unwrap());
    let mut registry = Registry::new();
    registry.register(Arc::new(source)).unwrap();
    let resolved = registry
        .resolve_frontend("guest-lang", "4", InvocationPolicy::ProtocolOnly)
        .unwrap();

    let config = HostConfig::new(PeerInfo {
        kind: PeerKind::Unspecified,
        name: "registry-test".into(),
        version: "1.0.0".into(),
    });
    let session = Session::open(resolved.connect(Path::new(".")).await.unwrap(), &config)
        .await
        .unwrap();
    assert_eq!(session.negotiated().extension().id, "guest");
    session.close().await.unwrap();
}

#[test]
fn registration_rejects_an_installed_source_that_claims_complete_scope() {
    let source = process_provider("installed-claims-complete", "gleam", "json")
        .as_ref()
        .clone()
        .with_scope(CapabilityMetadataScope::Complete);
    let mut registry = Registry::new();
    let error = registry.register(Arc::new(source)).unwrap_err().to_string();
    assert!(error.contains("installed-claims-complete"), "{error}");
    assert!(error.contains("Complete"), "{error}");
    assert!(error.contains("Installed"), "{error}");
}

#[test]
fn registration_rejects_a_builtin_source_that_claims_a_process_mode() {
    let source = builtin_alpha()
        .as_ref()
        .clone()
        .with_mode(InvocationMode::ProcessMep);
    let mut registry = Registry::new();
    let error = registry.register(Arc::new(source)).unwrap_err().to_string();
    assert!(error.contains("builtin-alpha"), "{error}");
    assert!(error.contains("ProcessMep"), "{error}");
    assert!(error.contains("Builtin"), "{error}");
}

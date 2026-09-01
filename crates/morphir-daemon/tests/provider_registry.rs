use morphir_common::home::MorphirHome;
use morphir_daemon::extensions::{
    CapabilityMetadataScope, ExtensionRegistry, InvocationMode, InvocationPolicy, ProviderOrigin,
};
use morphir_distribution::{
    Channel, ExtensionId, ExtensionInstaller, InstalledExtensionSnapshot, LocalIndex, Platform,
    Selection, Sha256Digest, list_installed,
};
use morphir_extension_sdk::protocol::{InitializeParams, PeerInfo};
use morphir_extension_sdk::{
    Backend, BackendCapability, CompileRequest, CompileResult, Extension, ExtensionCapabilities,
    ExtensionInfo, Frontend, FrontendCapability, GenerateRequest, GenerateResult,
    LanguageCapability, NativeExtension,
};
use std::fs;

macro_rules! native_provider {
    ($provider:ident, $id:literal, $language:literal, $target:literal, $ir_version:literal) => {
        #[derive(Default)]
        struct $provider;

        impl Extension for $provider {
            fn info() -> ExtensionInfo {
                ExtensionInfo {
                    id: $id.into(),
                    name: concat!("Test provider ", $id).into(),
                    version: "1.0.0".into(),
                    ..ExtensionInfo::default()
                }
            }

            fn capabilities() -> ExtensionCapabilities {
                ExtensionCapabilities {
                    frontend: Some(FrontendCapability {
                        languages: vec![LanguageCapability {
                            id: $language.into(),
                            file_extensions: vec![concat!(".", $language).into()],
                        }],
                        ir_versions: vec![$ir_version.into()],
                        compile: true,
                        incremental: false,
                        fragments: false,
                    }),
                    backend: Some(BackendCapability {
                        targets: vec![$target.into()],
                        ir_versions: vec![$ir_version.into()],
                        generate: true,
                    }),
                    ..ExtensionCapabilities::default()
                }
            }
        }

        impl Frontend for $provider {
            fn compile(
                &self,
                _request: CompileRequest,
            ) -> morphir_extension_sdk::Result<CompileResult> {
                Ok(CompileResult {
                    success: true,
                    ir_version: Some($ir_version.into()),
                    ir: None,
                    diagnostics: vec![],
                    modules: vec![],
                })
            }

            fn supported_languages() -> Vec<String> {
                vec![$language.into()]
            }

            fn file_extensions() -> Vec<String> {
                vec![concat!(".", $language).into()]
            }
        }

        impl Backend for $provider {
            fn generate(
                &self,
                _request: GenerateRequest,
            ) -> morphir_extension_sdk::Result<GenerateResult> {
                Ok(GenerateResult {
                    success: true,
                    artifacts: vec![],
                    diagnostics: vec![],
                })
            }

            fn target_languages() -> Vec<String> {
                vec![$target.into()]
            }
        }
    };
}

native_provider!(BuiltinAlpha, "builtin-alpha", "gleam", "json", "4.0.0");
native_provider!(BuiltinZulu, "builtin-zulu", "gleam", "json", "4.0.0");
native_provider!(AliasFour, "alias-four", "alias-lang", "alias-target", "4");
native_provider!(
    ExactFour,
    "exact-four",
    "exact-lang",
    "exact-target",
    "4.0.0"
);
native_provider!(SameId, "same-provider", "gleam", "json", "4");
native_provider!(WhitespaceIr, "whitespace-ir", "space", "space", " 4");
native_provider!(
    MalformedIr,
    "malformed-ir",
    "malformed",
    "malformed",
    "four"
);
native_provider!(UnsupportedIr, "unsupported-ir", "future", "future", "5");

#[derive(Default)]
struct EmptyVersions;

impl Extension for EmptyVersions {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "empty-versions".into(),
            name: "Empty versions".into(),
            version: "1.0.0".into(),
            ..ExtensionInfo::default()
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        let mut capabilities = ExactFour::capabilities();
        capabilities.frontend.as_mut().unwrap().ir_versions.clear();
        capabilities.backend.as_mut().unwrap().ir_versions.clear();
        capabilities
    }
}

impl Frontend for EmptyVersions {
    fn compile(&self, request: CompileRequest) -> morphir_extension_sdk::Result<CompileResult> {
        ExactFour.compile(request)
    }

    fn supported_languages() -> Vec<String> {
        ExactFour::supported_languages()
    }

    fn file_extensions() -> Vec<String> {
        ExactFour::file_extensions()
    }
}

impl Backend for EmptyVersions {
    fn generate(&self, request: GenerateRequest) -> morphir_extension_sdk::Result<GenerateResult> {
        ExactFour.generate(request)
    }

    fn target_languages() -> Vec<String> {
        ExactFour::target_languages()
    }
}

fn native<E>() -> NativeExtension
where
    E: Extension + Frontend + Backend + Send + Sync + Default + 'static,
{
    NativeExtension::frontend_backend(E::default()).unwrap()
}

#[derive(Clone, Copy)]
enum InstalledRuntime {
    Process,
    Wasm,
}

struct InstalledSpec<'a> {
    id: &'a str,
    runtime: InstalledRuntime,
    frontend: Option<(&'a str, &'a str, bool)>,
    backend: Option<(&'a str, &'a str, bool)>,
}

fn installed(spec: InstalledSpec<'_>) -> InstalledExtensionSnapshot {
    let root = tempfile::tempdir().unwrap();
    let index = root.path().join("index");
    let filename = match spec.runtime {
        InstalledRuntime::Process => spec.id.to_owned(),
        InstalledRuntime::Wasm => format!("{}.wasm", spec.id),
    };
    let source = index.join("artifacts").join(&filename);
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::create_dir_all(index.join("extensions")).unwrap();
    let bytes = match spec.runtime {
        InstalledRuntime::Process => b"#!/bin/sh\nexit 0\n".as_slice(),
        InstalledRuntime::Wasm => b"portable test wasm".as_slice(),
    };
    fs::write(&source, bytes).unwrap();
    let digest = Sha256Digest::of_bytes(bytes);
    let platform = Platform::current();
    let artifact = match spec.runtime {
        InstalledRuntime::Process => serde_json::json!({
            "runtime": "process",
            "platform": { "os": platform.os(), "arch": platform.arch() },
            "source": { "kind": "local-file", "path": format!("artifacts/{filename}") },
            "sha256": digest,
            "filename": filename,
            "args": [],
            "executable": true
        }),
        InstalledRuntime::Wasm => serde_json::json!({
            "runtime": "wasm",
            "source": { "kind": "local-file", "path": format!("artifacts/{filename}") },
            "sha256": digest,
            "filename": filename
        }),
    };
    let capabilities: Vec<&str> = [
        spec.frontend.map(|_| "frontend"),
        spec.backend.map(|_| "backend"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut record = serde_json::json!({
        "schemaVersion": if spec.frontend.is_some() { 3 } else { 2 },
        "id": spec.id,
        "name": format!("Installed {}", spec.id),
        "version": "2.0.0",
        "channels": ["stable"],
        "mepVersions": ["0.1"],
        "capabilities": capabilities,
        "artifacts": [artifact]
    });
    if let Some((language, ir_version, compile)) = spec.frontend {
        record.as_object_mut().unwrap().insert(
            "frontend".into(),
            serde_json::json!({
                "languages": [{
                    "id": language,
                    "fileExtensions": [format!(".{language}")]
                }],
                "irVersions": [ir_version],
                "compile": compile
            }),
        );
    }
    if let Some((target, ir_version, generate)) = spec.backend {
        record.as_object_mut().unwrap().insert(
            "backend".into(),
            serde_json::json!({
                "targets": [target],
                "irVersions": [ir_version],
                "generate": generate
            }),
        );
    }
    fs::write(
        index.join("extensions").join(format!("{}.jsonl", spec.id)),
        format!("{record}\n"),
    )
    .unwrap();

    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let id = ExtensionId::parse(spec.id).unwrap();
    let selected = LocalIndex::open(&index)
        .unwrap()
        .resolve(&id, Selection::Channel(Channel::Stable), &platform)
        .unwrap();
    ExtensionInstaller::new(&home).install(selected).unwrap();
    list_installed(&home).unwrap().pop().unwrap()
}

fn process_provider(id: &str, language: &str, target: &str) -> InstalledExtensionSnapshot {
    installed(InstalledSpec {
        id,
        runtime: InstalledRuntime::Process,
        frontend: Some((language, "4", true)),
        backend: Some((target, "4.0.0", true)),
    })
}

#[test]
fn resolves_frontend_and_backend_by_capability_instead_of_provider_id() {
    let mut registry = ExtensionRegistry::new();
    registry.register_builtin(native::<BuiltinAlpha>()).unwrap();

    let frontend = registry
        .resolve_frontend("gleam", "4", InvocationPolicy::PreferDirect)
        .unwrap();
    let backend = registry
        .resolve_backend("json", "4.0.0", InvocationPolicy::PreferDirect)
        .unwrap();

    assert_eq!(frontend.info().id, "builtin-alpha");
    assert_eq!(frontend.capability().languages[0].id, "gleam");
    assert_eq!(frontend.origin(), ProviderOrigin::Builtin);
    assert!(frontend.native_frontend().is_some());
    assert!(frontend.installed_snapshot().is_none());
    assert_eq!(backend.info().id, "builtin-alpha");
    assert_eq!(backend.capability().targets, ["json"]);
    assert!(backend.native_backend().is_some());
    assert!(backend.installed_snapshot().is_none());
}

#[test]
fn major_alias_and_exact_baseline_release_match_in_both_directions() {
    let mut registry = ExtensionRegistry::new();
    registry.register_builtin(native::<AliasFour>()).unwrap();
    registry.register_builtin(native::<ExactFour>()).unwrap();

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
        (native::<WhitespaceIr>(), " 4"),
        (native::<MalformedIr>(), "four"),
        (native::<UnsupportedIr>(), "5"),
    ] {
        let mut registry = ExtensionRegistry::new();
        let error = registry.register_builtin(provider).unwrap_err().to_string();
        assert!(error.contains("advertised"), "{error}");
        assert!(error.contains(expected), "{error}");
    }

    let mut registry = ExtensionRegistry::new();
    let error = registry
        .register_builtin(native::<EmptyVersions>())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("must advertise at least one frontend IR version"),
        "{error}"
    );
}

#[test]
fn malformed_and_unsupported_requested_ir_versions_are_rejected_before_resolution() {
    let mut registry = ExtensionRegistry::new();
    registry.register_builtin(native::<ExactFour>()).unwrap();

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
    let mut registry = ExtensionRegistry::new();
    registry.register_builtin(native::<BuiltinAlpha>()).unwrap();
    registry
        .register_installed(process_provider("installed-choice", "gleam", "json"))
        .unwrap();

    let frontend = registry
        .resolve_frontend("gleam", "4", InvocationPolicy::PreferDirect)
        .unwrap();
    let backend = registry
        .resolve_backend("json", "4", InvocationPolicy::PreferDirect)
        .unwrap();

    assert_eq!(frontend.info().id, "installed-choice");
    assert_eq!(frontend.origin(), ProviderOrigin::Installed);
    assert!(frontend.native_extension().is_none());
    assert!(frontend.installed_snapshot().is_some());
    assert_eq!(backend.info().id, "installed-choice");
    assert_eq!(backend.origin(), ProviderOrigin::Installed);
}

#[test]
fn nonmatching_installed_provider_does_not_suppress_matching_builtin() {
    let mut registry = ExtensionRegistry::new();
    registry.register_builtin(native::<BuiltinAlpha>()).unwrap();
    registry
        .register_installed(installed(InstalledSpec {
            id: "installed-other-version",
            runtime: InstalledRuntime::Process,
            frontend: Some(("gleam", "3", true)),
            backend: Some(("json", "3.0.0", true)),
        }))
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
    let mut registry = ExtensionRegistry::new();
    registry.register_builtin(native::<BuiltinZulu>()).unwrap();
    registry.register_builtin(native::<BuiltinAlpha>()).unwrap();

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
    let mut registry = ExtensionRegistry::new();
    registry.register_builtin(native::<SameId>()).unwrap();
    let error = registry
        .register_builtin(native::<SameId>())
        .unwrap_err()
        .to_string();
    assert!(error.contains("duplicate Builtin provider ID 'same-provider'"));

    registry
        .register_installed(process_provider("same-provider", "gleam", "json"))
        .unwrap();
    let duplicate_installed = process_provider("same-provider", "gleam", "json");
    let error = registry
        .register_installed(duplicate_installed)
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
fn invocation_policy_selects_direct_or_native_protocol_mode() {
    let mut registry = ExtensionRegistry::new();
    registry.register_builtin(native::<ExactFour>()).unwrap();

    let direct_frontend = registry
        .resolve_frontend("exact-lang", "4", InvocationPolicy::PreferDirect)
        .unwrap();
    assert_eq!(
        direct_frontend.invocation_mode(),
        InvocationMode::NativeDirect
    );
    assert!(direct_frontend.native_extension().is_some());
    assert!(direct_frontend.native_frontend().is_some());
    assert!(direct_frontend.native_mep_session().is_none());
    let protocol_backend = registry
        .resolve_backend("exact-target", "4", InvocationPolicy::ProtocolOnly)
        .unwrap();
    assert_eq!(
        protocol_backend.invocation_mode(),
        InvocationMode::NativeMep
    );
    assert!(protocol_backend.native_extension().is_none());
    assert!(protocol_backend.native_backend().is_none());
    assert!(protocol_backend.installed_snapshot().is_none());
    assert!(protocol_backend.native_mep_session().is_some());
}

#[tokio::test]
async fn protocol_only_resolution_exposes_a_native_mep_session_through_shutdown() {
    let mut registry = ExtensionRegistry::new();
    registry.register_builtin(native::<ExactFour>()).unwrap();

    let resolved = registry
        .resolve_frontend("exact-lang", "4", InvocationPolicy::ProtocolOnly)
        .unwrap();
    assert_eq!(resolved.invocation_mode(), InvocationMode::NativeMep);
    assert!(resolved.native_extension().is_none());
    assert!(resolved.native_frontend().is_none());
    assert!(resolved.installed_snapshot().is_none());

    let loaded = resolved
        .native_mep_session()
        .expect("NativeMep resolution should expose only a loaded protocol session");
    let ready = match loaded
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "registry-test".into(),
                version: "1.0.0".into(),
            },
        })
        .await
    {
        Ok(ready) => ready,
        Err(failure) => panic!("native MEP initialization failed: {}", failure.error()),
    };
    assert_eq!(ready.negotiated().extension().id, "exact-four");
    match ready.shutdown().await {
        Ok(_) => {}
        Err(failure) => panic!("native MEP shutdown failed: {}", failure.error()),
    }
}

#[test]
fn installed_runtime_selects_process_or_wasm_protocol_mode_regardless_of_policy() {
    let mut registry = ExtensionRegistry::new();
    registry
        .register_installed(process_provider(
            "process-provider",
            "process-lang",
            "process-target",
        ))
        .unwrap();
    registry
        .register_installed(installed(InstalledSpec {
            id: "wasm-provider",
            runtime: InstalledRuntime::Wasm,
            frontend: Some(("wasm-lang", "4", true)),
            backend: Some(("wasm-target", "4", true)),
        }))
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
    let mut registry = ExtensionRegistry::new();
    registry
        .register_installed(installed(InstalledSpec {
            id: "disabled-provider",
            runtime: InstalledRuntime::Process,
            frontend: Some(("disabled-lang", "4", false)),
            backend: Some(("disabled-target", "4", false)),
        }))
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
    let mut registry = ExtensionRegistry::new();
    registry.register_builtin(native::<ExactFour>()).unwrap();
    registry
        .register_installed(process_provider("installed-v4", "gleam", "json"))
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
    let mut registry = ExtensionRegistry::new();
    registry
        .register_installed(process_provider("installed-zulu", "gleam", "json"))
        .unwrap();
    registry.register_builtin(native::<ExactFour>()).unwrap();
    registry.register_builtin(native::<AliasFour>()).unwrap();

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
fn resolved_typed_views_expose_only_the_corresponding_native_handle() {
    let mut registry = ExtensionRegistry::new();
    registry.register_builtin(native::<ExactFour>()).unwrap();

    let frontend = registry
        .resolve_frontend("exact-lang", "4", InvocationPolicy::PreferDirect)
        .unwrap();
    let backend = registry
        .resolve_backend("exact-target", "4", InvocationPolicy::PreferDirect)
        .unwrap();

    assert!(frontend.native_frontend().is_some());
    assert_eq!(frontend.capability().languages[0].id, "exact-lang");
    assert!(backend.native_backend().is_some());
    assert_eq!(backend.capability().targets, ["exact-target"]);
}

#[test]
fn resolutions_and_listing_share_one_immutable_provider_entry() {
    let mut registry = ExtensionRegistry::new();
    registry.register_builtin(native::<ExactFour>()).unwrap();

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
        frontend.capability(),
        repeated_frontend.capability()
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
    let mut registry = ExtensionRegistry::new();
    registry.register_builtin(native::<ExactFour>()).unwrap();
    registry
        .register_installed(process_provider(
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

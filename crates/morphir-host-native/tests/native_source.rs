//! Registry behaviour that needs a real built-in, ported from the daemon's
//! `provider_registry` tests. The resolution rules themselves are tested in
//! `morphir-host/tests/registry.rs`.

use morphir_extension_sdk::protocol::{PeerInfo, PeerKind};
use morphir_extension_sdk::{
    Backend, BackendCapability, CompileRequest, CompileResult, Extension, ExtensionCapabilities,
    ExtensionInfo, ExtensionType, Frontend, FrontendCapability, GenerateRequest, GenerateResult,
    LanguageCapability, NativeExtension, Workspace, WorkspaceCapability,
};
use morphir_host::testing::FakeSource;
use morphir_host::{HostConfig, InvocationMode, InvocationPolicy, Registry, Session};
use morphir_host_native::NativeSource;
use std::path::Path;
use std::sync::Arc;

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
                        multi_document: false,
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
                    module_results: vec![],
                    context_digest: None,
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

native_provider!(AliasFour, "alias-four", "alias-lang", "alias-target", "4");
native_provider!(
    ExactFour,
    "exact-four",
    "exact-lang",
    "exact-target",
    "4.0.0"
);

fn native<E>() -> Arc<NativeSource>
where
    E: Extension + Frontend + Backend + Send + Sync + Default + 'static,
{
    Arc::new(NativeSource::new(
        NativeExtension::frontend_backend(E::default()).unwrap(),
    ))
}

fn workspace_capability() -> WorkspaceCapability {
    WorkspaceCapability {
        protocol_versions: vec![morphir_workspace::workspace_discovery_protocol()],
        discover: true,
    }
}

/// A frontend that also declares workspace discovery, so it can synthesize a
/// project from an explicit source selection.
#[derive(Default)]
struct SynthesizingFrontend;

impl Extension for SynthesizingFrontend {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "synthesizing".into(),
            name: "Synthesizing frontend".into(),
            version: "1.0.0".into(),
            ..ExtensionInfo::default()
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities {
            frontend: ExactFour::capabilities().frontend,
            workspace: Some(workspace_capability()),
            ..ExtensionCapabilities::default()
        }
    }
}

impl Frontend for SynthesizingFrontend {
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

impl Workspace for SynthesizingFrontend {
    fn discover(
        &self,
        request: morphir_workspace::DiscoveryRequest,
    ) -> morphir_extension_sdk::Result<morphir_workspace::DiscoveryResponse> {
        Ok(morphir_workspace::discover(request))
    }
}

/// An installed-like provider with one frontend language and one backend
/// target, as a process runtime would advertise them.
fn process_provider(id: &str, language: &str, target: &str) -> Arc<FakeSource> {
    Arc::new(FakeSource::installed(
        ExtensionInfo {
            id: id.into(),
            name: format!("Installed {id}"),
            version: "2.0.0".into(),
            types: vec![ExtensionType::Frontend, ExtensionType::Backend],
            ..ExtensionInfo::default()
        },
        ExtensionCapabilities {
            frontend: Some(FrontendCapability {
                languages: vec![LanguageCapability {
                    id: language.into(),
                    file_extensions: vec![format!(".{language}")],
                }],
                ir_versions: vec!["4".into()],
                compile: true,
                ..FrontendCapability::default()
            }),
            backend: Some(BackendCapability {
                targets: vec![target.into()],
                ir_versions: vec!["4.0.0".into()],
                generate: true,
            }),
            ..ExtensionCapabilities::default()
        },
        InvocationMode::ProcessMep,
    ))
}

#[test]
fn invocation_policy_selects_direct_or_native_protocol_mode() {
    let mut registry = Registry::new();
    registry.register(native::<ExactFour>()).unwrap();

    let direct_frontend = registry
        .resolve_frontend("exact-lang", "4", InvocationPolicy::PreferDirect)
        .unwrap();
    assert_eq!(
        direct_frontend.invocation_mode(),
        InvocationMode::NativeDirect
    );
    assert!(direct_frontend.native().is_some());
    assert!(
        direct_frontend
            .native()
            .and_then(NativeExtension::frontend)
            .is_some()
    );
    let protocol_backend = registry
        .resolve_backend("exact-target", "4", InvocationPolicy::ProtocolOnly)
        .unwrap();
    assert_eq!(
        protocol_backend.invocation_mode(),
        InvocationMode::NativeMep
    );
    // Under `NativeMep` the built-in stays behind `connect`, so a
    // protocol-only caller cannot skip MEP negotiation and result checks.
    assert!(protocol_backend.native().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn protocol_only_resolution_opens_a_native_mep_session_through_shutdown() {
    let mut registry = Registry::new();
    registry.register(native::<ExactFour>()).unwrap();

    let resolved = registry
        .resolve_frontend("exact-lang", "4", InvocationPolicy::ProtocolOnly)
        .unwrap();
    assert_eq!(resolved.invocation_mode(), InvocationMode::NativeMep);
    assert!(resolved.native().is_none());

    let connection = resolved
        .connect(Path::new("."))
        .await
        .expect("NativeMep resolution should connect to the built-in over the protocol");
    let config = HostConfig::new(PeerInfo {
        kind: PeerKind::Unspecified,
        name: "registry-test".into(),
        version: "1.0.0".into(),
    });
    let session = match Session::open(connection, &config).await {
        Ok(session) => session,
        Err(failure) => panic!("native MEP initialization failed: {failure}"),
    };
    assert_eq!(session.negotiated().extension().id, "exact-four");
    if let Err(failure) = session.close().await {
        panic!("native MEP shutdown failed: {failure}");
    }
}

#[test]
fn resolved_typed_views_expose_only_the_corresponding_native_handle() {
    let mut registry = Registry::new();
    registry.register(native::<ExactFour>()).unwrap();

    let frontend = registry
        .resolve_frontend("exact-lang", "4", InvocationPolicy::PreferDirect)
        .unwrap();
    let backend = registry
        .resolve_backend("exact-target", "4", InvocationPolicy::PreferDirect)
        .unwrap();

    assert!(
        frontend
            .native()
            .and_then(NativeExtension::frontend)
            .is_some()
    );
    assert_eq!(frontend.frontend().unwrap().languages[0].id, "exact-lang");
    assert!(frontend.backend().is_none());
    assert!(
        backend
            .native()
            .and_then(NativeExtension::backend)
            .is_some()
    );
    assert_eq!(backend.backend().unwrap().targets, ["exact-target"]);
    assert!(backend.frontend().is_none());
}

#[test]
fn a_resolved_frontend_reports_whether_it_can_synthesize() {
    let mut registry = Registry::new();
    registry
        .register(Arc::new(NativeSource::new(
            NativeExtension::builder(SynthesizingFrontend)
                .with_frontend()
                .with_workspace()
                .finish()
                .unwrap(),
        )))
        .unwrap();
    registry.register(native::<AliasFour>()).unwrap();
    registry
        .register(process_provider("installed-plain", "plain-lang", "plain"))
        .unwrap();

    let synthesizing = registry
        .resolve_frontend("exact-lang", "4", InvocationPolicy::PreferDirect)
        .unwrap();
    assert!(synthesizing.supports_workspace_discovery());
    assert!(
        synthesizing
            .native()
            .and_then(NativeExtension::workspace)
            .is_some()
    );

    let plain_builtin = registry
        .resolve_frontend("alias-lang", "4", InvocationPolicy::PreferDirect)
        .unwrap();
    assert!(!plain_builtin.supports_workspace_discovery());
    assert!(
        plain_builtin
            .native()
            .and_then(NativeExtension::workspace)
            .is_none()
    );

    let plain_installed = registry
        .resolve_frontend("plain-lang", "4", InvocationPolicy::PreferDirect)
        .unwrap();
    assert!(!plain_installed.supports_workspace_discovery());
}

use morphir_extension_sdk::native::doc_fixtures::DocFrontend;
use morphir_extension_sdk::protocol::{ExtensionRequest, PeerInfo, PeerKind, methods};
use morphir_extension_sdk::{
    Backend, BackendCapability, CompileOptions, CompilePackage, CompileRequest, CompileResult,
    Extension, ExtensionCapabilities, ExtensionInfo, ExtensionType, Frontend, FrontendCapability,
    GenerateRequest, GenerateResult, LanguageCapability, NativeExtension, SourceDocument,
    SourceSet, Workspace, WorkspaceCapability,
};
use morphir_host::{
    CallError, Channel, ChannelState, ExpectedChecks, HostConfig, HostError, JsonRpcConnection,
    Outgoing, Session,
};
use morphir_host_native::{CheckedConnection, NativeChannel};
use std::sync::{Arc, Mutex};

fn config() -> HostConfig {
    HostConfig::new(PeerInfo {
        kind: PeerKind::Unspecified,
        name: "test".into(),
        version: "1.0.0".into(),
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_native_frontend_compiles_through_a_session() {
    let extension = NativeExtension::frontend_only(DocFrontend).unwrap();
    let channel = NativeChannel::new(&extension);
    let checks = ExpectedChecks::new(channel.expectation());
    let connection = JsonRpcConnection::new(channel, checks);
    let mut session = Session::open(connection, &config()).await.unwrap();
    assert_eq!(session.negotiated().extension().id, extension.info().id);
    let result = session.compile(CompileRequest::default()).await;
    session.close().await.unwrap();
    assert!(result.is_ok());
}

// The native channel through a whole session, ported from the daemon's
// session tests.

/// A checked session over a native channel held to the extension's own
/// discovery metadata.
async fn open_native(extension: &NativeExtension) -> Session {
    let channel = NativeChannel::new(extension);
    let checks = ExpectedChecks::new(channel.expectation());
    let connection = CheckedConnection::new(JsonRpcConnection::new(channel, checks));
    Session::open(connection, &config())
        .await
        .unwrap_or_else(|error| panic!("native initialization failed: {error}"))
}

#[derive(Default)]
struct RecordingExtension {
    compile_requests: Arc<Mutex<Vec<CompileRequest>>>,
}

impl Extension for RecordingExtension {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "recording-native".into(),
            name: "Recording native extension".into(),
            version: "1.0.0".into(),
            ..ExtensionInfo::default()
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities {
            frontend: Some(FrontendCapability {
                languages: vec![LanguageCapability {
                    id: "recording".into(),
                    file_extensions: vec![".recording".into()],
                }],
                ir_versions: vec!["3".into()],
                compile: true,
                incremental: false,
                fragments: false,
                multi_document: false,
            }),
            backend: Some(BackendCapability {
                targets: vec!["recording".into()],
                ir_versions: vec!["3".into()],
                generate: true,
            }),
            ..ExtensionCapabilities::default()
        }
    }
}

impl Frontend for RecordingExtension {
    fn compile(&self, request: CompileRequest) -> morphir_extension_sdk::Result<CompileResult> {
        self.compile_requests.lock().unwrap().push(request.clone());
        Ok(successful_recording_compile_result(request))
    }

    fn supported_languages() -> Vec<String> {
        vec!["recording".into()]
    }

    fn file_extensions() -> Vec<String> {
        vec![".recording".into()]
    }
}

impl Backend for RecordingExtension {
    fn generate(&self, _request: GenerateRequest) -> morphir_extension_sdk::Result<GenerateResult> {
        Ok(GenerateResult {
            success: true,
            artifacts: vec![],
            diagnostics: vec![],
        })
    }

    fn target_languages() -> Vec<String> {
        vec!["recording".into()]
    }
}

#[derive(Default)]
struct ThreadRecordingExtension {
    compile_thread: Arc<Mutex<Option<std::thread::ThreadId>>>,
}

impl Extension for ThreadRecordingExtension {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "thread-recording-native".into(),
            name: "Thread recording native extension".into(),
            version: "1.0.0".into(),
            ..ExtensionInfo::default()
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        native_frontend_capabilities()
    }
}

impl Frontend for ThreadRecordingExtension {
    fn compile(&self, request: CompileRequest) -> morphir_extension_sdk::Result<CompileResult> {
        *self.compile_thread.lock().unwrap() = Some(std::thread::current().id());
        Ok(successful_recording_compile_result(request))
    }

    fn supported_languages() -> Vec<String> {
        vec!["recording".into()]
    }

    fn file_extensions() -> Vec<String> {
        vec![".recording".into()]
    }
}

struct PanickingExtension;

impl Extension for PanickingExtension {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "panicking-native".into(),
            name: "Panicking native extension".into(),
            version: "1.0.0".into(),
            ..ExtensionInfo::default()
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        native_frontend_capabilities()
    }
}

impl Frontend for PanickingExtension {
    fn compile(&self, _request: CompileRequest) -> morphir_extension_sdk::Result<CompileResult> {
        panic!("native compile panic for join-error coverage");
    }

    fn supported_languages() -> Vec<String> {
        vec!["recording".into()]
    }

    fn file_extensions() -> Vec<String> {
        vec![".recording".into()]
    }
}

fn native_frontend_capabilities() -> ExtensionCapabilities {
    ExtensionCapabilities {
        frontend: Some(FrontendCapability {
            languages: vec![LanguageCapability {
                id: "recording".into(),
                file_extensions: vec![".recording".into()],
            }],
            ir_versions: vec!["3".into()],
            compile: true,
            incremental: false,
            fragments: false,
            multi_document: false,
        }),
        ..ExtensionCapabilities::default()
    }
}

fn successful_recording_compile_result(request: CompileRequest) -> CompileResult {
    CompileResult {
        success: true,
        ir_version: Some(request.options.ir_version),
        ir: Some(serde_json::json!({
            "formatVersion": 3,
            "distribution": ["Library", [], [], {"modules": []}]
        })),
        diagnostics: vec![],
        modules: request.package.exposed_modules.unwrap_or_default(),
        module_results: vec![],
        context_digest: None,
    }
}

fn recording_compile_request() -> CompileRequest {
    CompileRequest {
        language_id: "recording".into(),
        sources: SourceSet {
            root: None,
            documents: vec![SourceDocument {
                uri: "file:///workspace/Example.recording".into(),
                language_id: "recording".into(),
                version: 1,
                text: "module Example".into(),
            }],
        },
        package: CompilePackage {
            name: "local/example".into(),
            exposed_modules: Some(vec!["Example".into()]),
        },
        dependencies: vec![],
        options: CompileOptions {
            types_only: false,
            ir_version: "3".into(),
            extra: Default::default(),
        },
        baseline: None,
    }
}

#[test]
fn native_transport_locks_exact_discovery_metadata_and_capabilities() {
    let native = NativeExtension::frontend_backend(RecordingExtension::default()).unwrap();
    let expected = NativeChannel::new(&native).expectation();

    assert_eq!(
        serde_json::to_value(expected.extension_info()).unwrap(),
        serde_json::to_value(Some(native.info())).unwrap()
    );
    assert_eq!(expected.capabilities(), Some(&native.capabilities()));
}

#[tokio::test(flavor = "multi_thread")]
async fn native_transport_runs_the_validated_mep_lifecycle() {
    let recording = RecordingExtension::default();
    let compile_requests = Arc::clone(&recording.compile_requests);
    let native = NativeExtension::frontend_backend(recording).unwrap();
    let mut session = open_native(&native).await;

    assert_eq!(session.negotiated().extension().id, native.info().id);
    assert_eq!(session.negotiated().capabilities(), &native.capabilities());

    let result = session
        .compile(recording_compile_request())
        .await
        .unwrap_or_else(|error| panic!("native compile failed: {error:?}"));
    assert!(result.success);
    assert_eq!(result.ir_version.as_deref(), Some("3"));
    assert_eq!(result.modules, ["Example"]);
    {
        let compile_requests = compile_requests.lock().unwrap();
        assert_eq!(compile_requests.len(), 1);
        assert_eq!(
            compile_requests[0].sources.documents[0].text,
            "module Example"
        );
    }

    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("native shutdown failed: {error}"));
}

#[tokio::test(flavor = "multi_thread")]
async fn native_transport_rejects_exchange_after_termination() {
    let native = NativeExtension::frontend_backend(RecordingExtension::default()).unwrap();
    let mut channel = NativeChannel::new(&native);

    assert_eq!(channel.close().await.unwrap(), ChannelState::Stopped);
    assert_eq!(channel.close().await.unwrap(), ChannelState::Stopped);
    let error = channel
        .send(Outgoing::Request(
            ExtensionRequest::new(methods::PING, serde_json::json!({}), 1).unwrap(),
        ))
        .await
        .expect_err("a stopped native channel must reject exchanges");

    assert_eq!(error.state, ChannelState::Stopped);
    assert!(error.message.contains("stopped"), "{}", error.message);
}

#[tokio::test(flavor = "current_thread")]
async fn native_transport_runs_protocol_handlers_off_the_async_worker() {
    let extension = ThreadRecordingExtension::default();
    let compile_thread = Arc::clone(&extension.compile_thread);
    let native = NativeExtension::frontend_only(extension).unwrap();
    let executor_thread = std::thread::current().id();
    let mut session = open_native(&native).await;

    session
        .compile(recording_compile_request())
        .await
        .unwrap_or_else(|error| panic!("native compile failed: {error:?}"));
    let handler_thread = compile_thread
        .lock()
        .unwrap()
        .expect("native compile handler should record its thread");

    assert_ne!(handler_thread, executor_thread);
    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("native shutdown failed: {error}"));
}

#[tokio::test(flavor = "multi_thread")]
async fn native_transport_reports_protocol_worker_panics_as_indeterminate() {
    let native = NativeExtension::frontend_only(PanickingExtension).unwrap();
    let mut session = open_native(&native).await;

    match session.compile(recording_compile_request()).await {
        Err(CallError::Failed(HostError::Channel { message, state, .. })) => {
            assert_eq!(
                state,
                ChannelState::Indeterminate,
                "a protocol worker panic must be indeterminate"
            );
            assert!(
                message.contains("Native extension protocol worker failed"),
                "{message}"
            );
            assert!(message.contains("panicked"), "{message}");
        }
        other => panic!("a panicking native compile must fail the session: {other:?}"),
    }
}

/// A workspace-only native extension, built the way `with_workspace()` is
/// documented to be used (`NativeExtension::builder(x).with_workspace().finish()`),
/// exercised through a real native session below rather than a scripted
/// channel.
struct RecordingWorkspaceExtension;

impl Extension for RecordingWorkspaceExtension {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "recording-workspace-native".into(),
            name: "Recording workspace native extension".into(),
            version: "1.0.0".into(),
            ..ExtensionInfo::default()
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities {
            workspace: Some(WorkspaceCapability {
                protocol_versions: vec![morphir_workspace::workspace_discovery_protocol()],
                discover: true,
            }),
            ..ExtensionCapabilities::default()
        }
    }
}

impl Workspace for RecordingWorkspaceExtension {
    fn discover(
        &self,
        request: morphir_workspace::DiscoveryRequest,
    ) -> morphir_extension_sdk::Result<morphir_workspace::DiscoveryResponse> {
        Ok(morphir_workspace::discover(request))
    }
}

fn native_workspace_discovery_request() -> morphir_workspace::DiscoveryRequest {
    use morphir_workspace::{FileEntry, FileTree, RelativePath};
    morphir_workspace::DiscoveryRequest {
        protocol_version: morphir_workspace::workspace_discovery_protocol(),
        development_root: FileTree {
            entries: std::collections::BTreeMap::from([
                (RelativePath::root(), FileEntry::Directory),
                (
                    RelativePath::parse("morphir.toml").unwrap(),
                    FileEntry::File {
                        text: "[project]\nname = \"acme/orders\"\n".into(),
                    },
                ),
            ]),
        },
        morphir_home: None,
        system_config: None,
        environment: std::collections::BTreeMap::new(),
        cli_overlay: serde_json::json!({}),
        purpose: Default::default(),
    }
}

/// A workspace-bearing `NativeExtension`, connected through a real native
/// channel, negotiated and invoked for `morphir.workspace.discover`,
/// producing a genuine `DiscoveryResponse` rather than a scripted one. The
/// provider registry is never consulted on this path.
#[tokio::test(flavor = "multi_thread")]
async fn native_transport_runs_workspace_discovery_through_a_real_session() {
    let native = NativeExtension::builder(RecordingWorkspaceExtension)
        .with_workspace()
        .finish()
        .unwrap();
    let mut session = open_native(&native).await;

    assert_eq!(session.negotiated().extension().id, native.info().id);
    assert!(
        session
            .negotiated()
            .extension()
            .types
            .contains(&ExtensionType::Workspace)
    );
    assert_eq!(session.negotiated().capabilities(), &native.capabilities());

    let response = session
        .call::<_, morphir_workspace::DiscoveryResponse>(
            methods::WORKSPACE_DISCOVER,
            native_workspace_discovery_request(),
        )
        .await
        .unwrap_or_else(|error| panic!("workspace discovery failed: {error:?}"));
    let snapshot = response
        .into_result()
        .expect("the workspace fixture should discover a project");
    assert_eq!(snapshot.projects.len(), 1);
    assert_eq!(snapshot.projects[0].name, "acme/orders");

    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("native shutdown failed: {error}"));
}

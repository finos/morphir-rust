//! Native adapters for invoking extension implementations without WASM.
//!
//! [`NativeExtension`] makes a single extension instance available through
//! typed frontend/backend handles and through the Morphir Extension Protocol.

use crate::protocol::{ExtensionRequest, ExtensionResponse};
use crate::{
    __dispatch_backend, __dispatch_frontend, __dispatch_request_with_metadata, __extension_info,
    Backend, CompileRequest, CompileResult, DispatchFn, Extension, ExtensionCapabilities,
    ExtensionError, ExtensionInfo, ExtensionType, Frontend, GenerateRequest, GenerateResult,
    Result,
};
use std::sync::Arc;

/// A typed native frontend endpoint.
pub trait NativeFrontend: Send + Sync {
    /// Compile a typed request without serializing through the protocol.
    fn compile(&self, request: CompileRequest) -> Result<CompileResult>;
}

/// A typed native backend endpoint.
pub trait NativeBackend: Send + Sync {
    /// Generate a typed request without serializing through the protocol.
    fn generate(&self, request: GenerateRequest) -> Result<GenerateResult>;
}

/// A native Morphir Extension Protocol endpoint.
pub trait NativeProtocol: Send + Sync {
    /// Handle one JSON-RPC extension request.
    fn handle(&self, request: ExtensionRequest) -> ExtensionResponse;
}

/// An extension implementation exposed through native typed and protocol APIs.
#[derive(Clone)]
pub struct NativeExtension {
    info: ExtensionInfo,
    capabilities: ExtensionCapabilities,
    frontend: Option<Arc<dyn NativeFrontend>>,
    backend: Option<Arc<dyn NativeBackend>>,
    protocol: Arc<dyn NativeProtocol>,
}

impl NativeExtension {
    /// Expose an extension that provides both frontend and backend capabilities.
    pub fn frontend_backend<E>(extension: E) -> Result<Self>
    where
        E: Extension + Frontend + Backend + Send + Sync + 'static,
    {
        let extension = Arc::new(extension);
        let frontend = Arc::new(FrontendHandle {
            extension: Arc::clone(&extension),
        }) as Arc<dyn NativeFrontend>;
        let backend = Arc::new(BackendHandle {
            extension: Arc::clone(&extension),
        }) as Arc<dyn NativeBackend>;
        Self::with_extension(
            extension,
            vec![ExtensionType::Frontend, ExtensionType::Backend],
            vec![__dispatch_frontend::<E>, __dispatch_backend::<E>],
            Some(frontend),
            Some(backend),
        )
    }

    /// Expose an extension that provides only a frontend capability.
    pub fn frontend_only<E>(extension: E) -> Result<Self>
    where
        E: Extension + Frontend + Send + Sync + 'static,
    {
        let extension = Arc::new(extension);
        let frontend = Arc::new(FrontendHandle {
            extension: Arc::clone(&extension),
        }) as Arc<dyn NativeFrontend>;
        Self::with_extension(
            extension,
            vec![ExtensionType::Frontend],
            vec![__dispatch_frontend::<E>],
            Some(frontend),
            None,
        )
    }

    /// Expose an extension that provides only a backend capability.
    pub fn backend_only<E>(extension: E) -> Result<Self>
    where
        E: Extension + Backend + Send + Sync + 'static,
    {
        let extension = Arc::new(extension);
        let backend = Arc::new(BackendHandle {
            extension: Arc::clone(&extension),
        }) as Arc<dyn NativeBackend>;
        Self::with_extension(
            extension,
            vec![ExtensionType::Backend],
            vec![__dispatch_backend::<E>],
            None,
            Some(backend),
        )
    }

    fn with_extension<E>(
        extension: Arc<E>,
        declared_types: Vec<ExtensionType>,
        dispatchers: Vec<DispatchFn<E>>,
        frontend: Option<Arc<dyn NativeFrontend>>,
        backend: Option<Arc<dyn NativeBackend>>,
    ) -> Result<Self>
    where
        E: Extension + Send + Sync + 'static,
    {
        let info = __extension_info::<E>(&declared_types);
        let capabilities = E::capabilities();
        validate_capabilities(&info, &capabilities, &declared_types)?;
        validate_protocol_metadata(&info, &capabilities)?;
        let protocol = Arc::new(ProtocolHandle {
            extension,
            dispatchers,
            info: info.clone(),
            capabilities: capabilities.clone(),
        });

        Ok(Self {
            info,
            capabilities,
            frontend,
            backend,
            protocol,
        })
    }

    /// Return the extension metadata, including the declared native handles.
    pub fn info(&self) -> &ExtensionInfo {
        &self.info
    }

    /// Return the extension's advertised capabilities.
    pub fn capabilities(&self) -> &ExtensionCapabilities {
        &self.capabilities
    }

    /// Return the direct frontend handle when the provider exposes one.
    pub fn frontend(&self) -> Option<&dyn NativeFrontend> {
        self.frontend.as_deref()
    }

    /// Return the direct backend handle when the provider exposes one.
    pub fn backend(&self) -> Option<&dyn NativeBackend> {
        self.backend.as_deref()
    }

    /// Return the protocol endpoint.
    pub fn protocol(&self) -> &dyn NativeProtocol {
        self.protocol.as_ref()
    }
}

fn validate_capabilities(
    info: &ExtensionInfo,
    capabilities: &ExtensionCapabilities,
    declared_types: &[ExtensionType],
) -> Result<()> {
    if capabilities.workspace.is_some() {
        return Err(ExtensionError::InitFailed(
            "extension advertises workspace without a native workspace handle".into(),
        ));
    }

    let has_frontend_handle = declared_types.contains(&ExtensionType::Frontend);
    match (has_frontend_handle, capabilities.frontend.as_ref()) {
        (true, Some(frontend)) if frontend.compile => {}
        (true, _) => {
            return Err(ExtensionError::UnsupportedCapability {
                extension: info.id.clone(),
                capability: "frontend.compile".into(),
            });
        }
        (false, None) => {}
        (false, Some(_)) => {
            return Err(ExtensionError::InitFailed(
                "extension advertises frontend without a native frontend handle".into(),
            ));
        }
    }

    let has_backend_handle = declared_types.contains(&ExtensionType::Backend);
    match (has_backend_handle, capabilities.backend.as_ref()) {
        (true, Some(backend)) if backend.generate => Ok(()),
        (true, _) => Err(ExtensionError::UnsupportedCapability {
            extension: info.id.clone(),
            capability: "backend.generate".into(),
        }),
        (false, None) => Ok(()),
        (false, Some(_)) => Err(ExtensionError::InitFailed(
            "extension advertises backend without a native backend handle".into(),
        )),
    }
}

fn validate_protocol_metadata(
    info: &ExtensionInfo,
    capabilities: &ExtensionCapabilities,
) -> Result<()> {
    serde_json::to_value(info).map_err(|error| {
        ExtensionError::InitFailed(format!(
            "extension info cannot be serialized for protocol discovery: {error}"
        ))
    })?;
    serde_json::to_value(capabilities).map_err(|error| {
        ExtensionError::InitFailed(format!(
            "extension capabilities cannot be serialized for protocol discovery: {error}"
        ))
    })?;
    Ok(())
}

struct FrontendHandle<E> {
    extension: Arc<E>,
}

impl<E> NativeFrontend for FrontendHandle<E>
where
    E: Frontend + Send + Sync,
{
    fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
        self.extension.compile(request)
    }
}

struct BackendHandle<E> {
    extension: Arc<E>,
}

impl<E> NativeBackend for BackendHandle<E>
where
    E: Backend + Send + Sync,
{
    fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
        self.extension.generate(request)
    }
}

struct ProtocolHandle<E> {
    extension: Arc<E>,
    dispatchers: Vec<DispatchFn<E>>,
    info: ExtensionInfo,
    capabilities: ExtensionCapabilities,
}

impl<E> NativeProtocol for ProtocolHandle<E>
where
    E: Extension + Send + Sync,
{
    fn handle(&self, request: ExtensionRequest) -> ExtensionResponse {
        __dispatch_request_with_metadata(
            self.extension.as_ref(),
            &request,
            &self.dispatchers,
            &self.info,
            &self.capabilities,
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::ExtensionError;
    use crate::NativeExtension;
    use crate::protocol::{ExtensionRequest, methods};
    use crate::{
        Artifact, Backend, BackendCapability, CompileOptions, CompilePackage, CompileRequest,
        CompileResult, Extension, ExtensionCapabilities, ExtensionInfo, ExtensionType, Frontend,
        FrontendCapability, GenerateRequest, GenerateResult, LanguageCapability, Result,
        SourceDocument, WorkspaceCapability,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    static SNAPSHOT_INFO_CALLS: AtomicUsize = AtomicUsize::new(0);
    static SNAPSHOT_CAPABILITY_CALLS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Clone, Default)]
    struct RecordingExtension {
        compile_requests: Arc<Mutex<Vec<CompileRequest>>>,
    }

    impl Extension for RecordingExtension {
        fn info() -> ExtensionInfo {
            ExtensionInfo {
                id: "recording".into(),
                name: "Recording extension".into(),
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
        fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
            self.compile_requests.lock().unwrap().push(request.clone());
            Ok(CompileResult {
                success: true,
                ir_version: Some(request.options.ir_version),
                ir: Some(serde_json::json!({ "typed": request.documents[0].text })),
                diagnostics: vec![],
                modules: request.package.exposed_modules,
            })
        }

        fn supported_languages() -> Vec<String> {
            vec!["recording".into()]
        }

        fn file_extensions() -> Vec<String> {
            vec![".recording".into()]
        }
    }

    impl Backend for RecordingExtension {
        fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
            Ok(GenerateResult {
                success: true,
                artifacts: vec![Artifact {
                    path: "recording.txt".into(),
                    content: request.ir.to_string(),
                    binary: false,
                }],
                diagnostics: vec![],
            })
        }

        fn target_languages() -> Vec<String> {
            vec!["recording".into()]
        }
    }

    #[derive(Default)]
    struct FrontendOnly;

    impl Extension for FrontendOnly {
        fn info() -> ExtensionInfo {
            ExtensionInfo::default()
        }

        fn capabilities() -> ExtensionCapabilities {
            ExtensionCapabilities {
                frontend: Some(FrontendCapability {
                    compile: true,
                    ..FrontendCapability::default()
                }),
                ..ExtensionCapabilities::default()
            }
        }
    }

    impl Frontend for FrontendOnly {
        fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
            Ok(CompileResult {
                success: true,
                ir_version: None,
                ir: None,
                diagnostics: vec![],
                modules: request.package.exposed_modules,
            })
        }

        fn supported_languages() -> Vec<String> {
            vec![]
        }

        fn file_extensions() -> Vec<String> {
            vec![]
        }
    }

    #[derive(Default)]
    struct BackendOnly;

    impl Extension for BackendOnly {
        fn info() -> ExtensionInfo {
            ExtensionInfo::default()
        }

        fn capabilities() -> ExtensionCapabilities {
            ExtensionCapabilities {
                backend: Some(BackendCapability {
                    generate: true,
                    ..BackendCapability::default()
                }),
                ..ExtensionCapabilities::default()
            }
        }
    }

    impl Backend for BackendOnly {
        fn generate(&self, _request: GenerateRequest) -> Result<GenerateResult> {
            Ok(GenerateResult {
                success: true,
                artifacts: vec![],
                diagnostics: vec![],
            })
        }

        fn target_languages() -> Vec<String> {
            vec![]
        }
    }

    #[derive(Default)]
    struct FrontendWithoutCompile;

    impl Extension for FrontendWithoutCompile {
        fn info() -> ExtensionInfo {
            ExtensionInfo::default()
        }

        fn capabilities() -> ExtensionCapabilities {
            ExtensionCapabilities {
                frontend: Some(FrontendCapability::default()),
                ..ExtensionCapabilities::default()
            }
        }
    }

    impl Frontend for FrontendWithoutCompile {
        fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
            FrontendOnly.compile(request)
        }

        fn supported_languages() -> Vec<String> {
            vec![]
        }

        fn file_extensions() -> Vec<String> {
            vec![]
        }
    }

    #[derive(Default)]
    struct BackendWithoutGenerate;

    impl Extension for BackendWithoutGenerate {
        fn info() -> ExtensionInfo {
            ExtensionInfo::default()
        }

        fn capabilities() -> ExtensionCapabilities {
            ExtensionCapabilities {
                backend: Some(BackendCapability::default()),
                ..ExtensionCapabilities::default()
            }
        }
    }

    impl Backend for BackendWithoutGenerate {
        fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
            BackendOnly.generate(request)
        }

        fn target_languages() -> Vec<String> {
            vec![]
        }
    }

    struct SnapshotExtension;

    impl Extension for SnapshotExtension {
        fn info() -> ExtensionInfo {
            let call = SNAPSHOT_INFO_CALLS.fetch_add(1, Ordering::SeqCst);
            ExtensionInfo {
                id: format!("snapshot-info-{call}"),
                ..ExtensionInfo::default()
            }
        }

        fn capabilities() -> ExtensionCapabilities {
            let call = SNAPSHOT_CAPABILITY_CALLS.fetch_add(1, Ordering::SeqCst);
            ExtensionCapabilities {
                frontend: Some(FrontendCapability {
                    ir_versions: vec![format!("snapshot-capability-{call}")],
                    compile: true,
                    ..FrontendCapability::default()
                }),
                ..ExtensionCapabilities::default()
            }
        }
    }

    impl Frontend for SnapshotExtension {
        fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
            FrontendOnly.compile(request)
        }

        fn supported_languages() -> Vec<String> {
            vec![]
        }

        fn file_extensions() -> Vec<String> {
            vec![]
        }
    }

    struct StatefulExtension {
        compile_calls: Mutex<usize>,
    }

    impl Extension for StatefulExtension {
        fn info() -> ExtensionInfo {
            ExtensionInfo::default()
        }

        fn capabilities() -> ExtensionCapabilities {
            ExtensionCapabilities {
                frontend: Some(FrontendCapability {
                    compile: true,
                    ..FrontendCapability::default()
                }),
                ..ExtensionCapabilities::default()
            }
        }
    }

    impl Frontend for StatefulExtension {
        fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
            let mut compile_calls = self.compile_calls.lock().unwrap();
            *compile_calls += 1;
            Ok(CompileResult {
                success: true,
                ir_version: Some(request.options.ir_version),
                ir: Some(serde_json::json!({ "call": *compile_calls })),
                diagnostics: vec![],
                modules: request.package.exposed_modules,
            })
        }

        fn supported_languages() -> Vec<String> {
            vec![]
        }

        fn file_extensions() -> Vec<String> {
            vec![]
        }
    }

    #[derive(Default)]
    struct FrontendWithWorkspace;

    impl Extension for FrontendWithWorkspace {
        fn info() -> ExtensionInfo {
            ExtensionInfo::default()
        }

        fn capabilities() -> ExtensionCapabilities {
            ExtensionCapabilities {
                frontend: Some(FrontendCapability {
                    compile: true,
                    ..FrontendCapability::default()
                }),
                workspace: Some(WorkspaceCapability {
                    protocol_versions: vec![1],
                    discover: true,
                }),
                ..ExtensionCapabilities::default()
            }
        }
    }

    impl Frontend for FrontendWithWorkspace {
        fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
            FrontendOnly.compile(request)
        }

        fn supported_languages() -> Vec<String> {
            vec![]
        }

        fn file_extensions() -> Vec<String> {
            vec![]
        }
    }

    #[derive(Default)]
    struct BackendWithWorkspace;

    impl Extension for BackendWithWorkspace {
        fn info() -> ExtensionInfo {
            ExtensionInfo::default()
        }

        fn capabilities() -> ExtensionCapabilities {
            ExtensionCapabilities {
                backend: Some(BackendCapability {
                    generate: true,
                    ..BackendCapability::default()
                }),
                workspace: Some(WorkspaceCapability {
                    protocol_versions: vec![1],
                    discover: true,
                }),
                ..ExtensionCapabilities::default()
            }
        }
    }

    impl Backend for BackendWithWorkspace {
        fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
            BackendOnly.generate(request)
        }

        fn target_languages() -> Vec<String> {
            vec![]
        }
    }

    #[derive(Default)]
    struct FrontendBackendWithWorkspace;

    impl Extension for FrontendBackendWithWorkspace {
        fn info() -> ExtensionInfo {
            ExtensionInfo::default()
        }

        fn capabilities() -> ExtensionCapabilities {
            ExtensionCapabilities {
                frontend: Some(FrontendCapability {
                    compile: true,
                    ..FrontendCapability::default()
                }),
                backend: Some(BackendCapability {
                    generate: true,
                    ..BackendCapability::default()
                }),
                workspace: Some(WorkspaceCapability {
                    protocol_versions: vec![1],
                    discover: true,
                }),
                ..ExtensionCapabilities::default()
            }
        }
    }

    impl Frontend for FrontendBackendWithWorkspace {
        fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
            FrontendOnly.compile(request)
        }

        fn supported_languages() -> Vec<String> {
            vec![]
        }

        fn file_extensions() -> Vec<String> {
            vec![]
        }
    }

    impl Backend for FrontendBackendWithWorkspace {
        fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
            BackendOnly.generate(request)
        }

        fn target_languages() -> Vec<String> {
            vec![]
        }
    }

    struct MetadataExtension<const FRONTEND: bool, const BACKEND: bool, const RESERVED_EXTRA: bool>;

    impl<const FRONTEND: bool, const BACKEND: bool, const RESERVED_EXTRA: bool> Extension
        for MetadataExtension<FRONTEND, BACKEND, RESERVED_EXTRA>
    {
        fn info() -> ExtensionInfo {
            ExtensionInfo {
                id: "metadata-extension".into(),
                name: "Metadata extension".into(),
                ..ExtensionInfo::default()
            }
        }

        fn capabilities() -> ExtensionCapabilities {
            let extra_key = if RESERVED_EXTRA {
                "streaming"
            } else {
                "experimental"
            };
            ExtensionCapabilities {
                frontend: FRONTEND.then(|| FrontendCapability {
                    compile: true,
                    ..FrontendCapability::default()
                }),
                backend: BACKEND.then(|| BackendCapability {
                    generate: true,
                    ..BackendCapability::default()
                }),
                extra: [(extra_key.into(), serde_json::json!({ "enabled": true }))]
                    .into_iter()
                    .collect(),
                ..ExtensionCapabilities::default()
            }
        }
    }

    impl<const FRONTEND: bool, const BACKEND: bool, const RESERVED_EXTRA: bool> Frontend
        for MetadataExtension<FRONTEND, BACKEND, RESERVED_EXTRA>
    {
        fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
            FrontendOnly.compile(request)
        }

        fn supported_languages() -> Vec<String> {
            vec![]
        }

        fn file_extensions() -> Vec<String> {
            vec![]
        }
    }

    impl<const FRONTEND: bool, const BACKEND: bool, const RESERVED_EXTRA: bool> Backend
        for MetadataExtension<FRONTEND, BACKEND, RESERVED_EXTRA>
    {
        fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
            BackendOnly.generate(request)
        }

        fn target_languages() -> Vec<String> {
            vec![]
        }
    }

    fn compile_request(source: &str) -> CompileRequest {
        CompileRequest {
            language_id: "recording".into(),
            documents: vec![SourceDocument {
                uri: "file:///workspace/Example.recording".into(),
                language_id: "recording".into(),
                version: 1,
                text: source.into(),
            }],
            package: CompilePackage {
                name: "local/example".into(),
                exposed_modules: vec!["Example".into()],
            },
            dependencies: vec![],
            options: CompileOptions {
                types_only: false,
                ir_version: "3".into(),
                extra: Default::default(),
            },
        }
    }

    #[test]
    fn direct_and_protocol_frontend_dispatch_are_equivalent() {
        let extension = RecordingExtension::default();
        let recorded_requests = extension.compile_requests.clone();
        let provider = NativeExtension::frontend_backend(extension).unwrap();
        let request = compile_request("pub fn hello() { \"world\" }");

        let direct = provider
            .frontend()
            .unwrap()
            .compile(request.clone())
            .unwrap();
        let rpc = ExtensionRequest::new(methods::COMPILE, request.clone(), 7).unwrap();
        let protocol = provider.protocol().handle(rpc);
        let through_mep: CompileResult = serde_json::from_value(protocol.result.unwrap()).unwrap();

        let direct_result = serde_json::to_value(&direct).unwrap();
        let protocol_result = serde_json::to_value(&through_mep).unwrap();
        assert_eq!(direct_result, protocol_result);
        let recorded_requests = recorded_requests.lock().unwrap();
        assert_eq!(recorded_requests.len(), 2);
        assert!(recorded_requests.iter().all(|recorded| {
            recorded.language_id == request.language_id
                && recorded.documents[0].text == request.documents[0].text
                && recorded.package.exposed_modules == request.package.exposed_modules
                && recorded.options.ir_version == request.options.ir_version
        }));
        assert_eq!(
            provider.info().types,
            [ExtensionType::Frontend, ExtensionType::Backend]
        );
    }

    #[test]
    fn frontend_only_provider_has_no_backend_handle() {
        let provider = NativeExtension::frontend_only(FrontendOnly).unwrap();

        assert!(provider.frontend().is_some());
        assert!(provider.backend().is_none());
    }

    #[test]
    fn backend_only_provider_has_no_frontend_handle() {
        let provider = NativeExtension::backend_only(BackendOnly).unwrap();

        assert!(provider.frontend().is_none());
        assert!(provider.backend().is_some());
    }

    #[test]
    fn frontend_only_rejects_missing_compile_capability() {
        let result = NativeExtension::frontend_only(FrontendWithoutCompile);

        assert!(matches!(
            result,
            Err(ExtensionError::UnsupportedCapability { capability, .. })
                if capability == "frontend.compile"
        ));
    }

    #[test]
    fn backend_only_rejects_missing_generate_capability() {
        let result = NativeExtension::backend_only(BackendWithoutGenerate);

        assert!(matches!(
            result,
            Err(ExtensionError::UnsupportedCapability { capability, .. })
                if capability == "backend.generate"
        ));
    }

    #[test]
    fn frontend_only_rejects_an_advertised_backend_without_a_handle() {
        let result = NativeExtension::frontend_only(RecordingExtension::default());

        assert!(matches!(
            result,
            Err(ExtensionError::InitFailed(message))
                if message == "extension advertises backend without a native backend handle"
        ));
    }

    #[test]
    fn native_protocol_discovery_uses_construction_metadata_snapshots() {
        SNAPSHOT_INFO_CALLS.store(0, Ordering::SeqCst);
        SNAPSHOT_CAPABILITY_CALLS.store(0, Ordering::SeqCst);
        let provider = NativeExtension::frontend_only(SnapshotExtension).unwrap();
        let expected_info = serde_json::to_value(provider.info()).unwrap();
        let expected_capabilities = serde_json::to_value(provider.capabilities()).unwrap();

        let info = provider
            .protocol()
            .handle(ExtensionRequest::new(methods::INFO, serde_json::json!({}), 1).unwrap());
        assert_eq!(info.result.unwrap(), expected_info);

        let capabilities = provider.protocol().handle(
            ExtensionRequest::new(methods::CAPABILITIES, serde_json::json!({}), 2).unwrap(),
        );
        assert_eq!(capabilities.result.unwrap(), expected_capabilities);

        let initialize = provider.protocol().handle(
            ExtensionRequest::new(
                methods::INITIALIZE,
                crate::protocol::InitializeParams {
                    protocol_versions: vec![crate::protocol::MEP_VERSION.into()],
                    host: crate::protocol::PeerInfo {
                        name: "test-host".into(),
                        version: "1.0.0".into(),
                    },
                },
                3,
            )
            .unwrap(),
        );
        let initialize: crate::protocol::InitializeResult =
            serde_json::from_value(initialize.result.unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(initialize.extension).unwrap(),
            expected_info
        );
        assert_eq!(
            serde_json::to_value(initialize.capabilities).unwrap(),
            expected_capabilities
        );
        assert_eq!(SNAPSHOT_INFO_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(SNAPSHOT_CAPABILITY_CALLS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn direct_and_protocol_calls_share_one_non_clone_extension_instance() {
        let provider = NativeExtension::frontend_only(StatefulExtension {
            compile_calls: Mutex::new(0),
        })
        .unwrap();
        let request = compile_request("pub fn shared() {}");

        let direct = provider
            .frontend()
            .unwrap()
            .compile(request.clone())
            .unwrap();
        let protocol = provider
            .protocol()
            .handle(ExtensionRequest::new(methods::COMPILE, request, 1).unwrap());
        let protocol: CompileResult = serde_json::from_value(protocol.result.unwrap()).unwrap();

        assert_eq!(direct.ir, Some(serde_json::json!({ "call": 1 })));
        assert_eq!(protocol.ir, Some(serde_json::json!({ "call": 2 })));
    }

    #[test]
    fn native_constructors_reject_advertised_workspace_without_a_handle() {
        let results = [
            NativeExtension::frontend_only(FrontendWithWorkspace),
            NativeExtension::backend_only(BackendWithWorkspace),
            NativeExtension::frontend_backend(FrontendBackendWithWorkspace),
        ];

        for result in results {
            assert!(matches!(
                result,
                Err(ExtensionError::InitFailed(message))
                    if message == "extension advertises workspace without a native workspace handle"
            ));
        }
    }

    #[test]
    fn native_constructors_reject_capabilities_that_protocol_discovery_cannot_serialize() {
        let results = [
            NativeExtension::frontend_only(MetadataExtension::<true, false, true>),
            NativeExtension::backend_only(MetadataExtension::<false, true, true>),
            NativeExtension::frontend_backend(MetadataExtension::<true, true, true>),
        ];

        for result in results {
            assert!(matches!(
                result,
                Err(ExtensionError::InitFailed(message))
                    if message.contains("capabilities cannot be serialized for protocol discovery")
                        && message.contains("reserved capability key 'streaming'")
            ));
        }
    }

    #[test]
    fn serializable_metadata_matches_all_protocol_discovery_responses() {
        let provider =
            NativeExtension::frontend_backend(MetadataExtension::<true, true, false>).unwrap();
        let expected_info = serde_json::to_value(provider.info()).unwrap();
        let expected_capabilities = serde_json::to_value(provider.capabilities()).unwrap();
        assert_eq!(
            expected_capabilities["experimental"],
            serde_json::json!({ "enabled": true })
        );

        let info = provider
            .protocol()
            .handle(ExtensionRequest::new(methods::INFO, serde_json::json!({}), 1).unwrap());
        assert_eq!(info.result.unwrap(), expected_info);

        let capabilities = provider.protocol().handle(
            ExtensionRequest::new(methods::CAPABILITIES, serde_json::json!({}), 2).unwrap(),
        );
        assert_eq!(capabilities.result.unwrap(), expected_capabilities);

        let initialize = provider.protocol().handle(
            ExtensionRequest::new(
                methods::INITIALIZE,
                crate::protocol::InitializeParams {
                    protocol_versions: vec![crate::protocol::MEP_VERSION.into()],
                    host: crate::protocol::PeerInfo {
                        name: "test-host".into(),
                        version: "1.0.0".into(),
                    },
                },
                3,
            )
            .unwrap(),
        );
        let initialize: crate::protocol::InitializeResult =
            serde_json::from_value(initialize.result.unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(initialize.extension).unwrap(),
            expected_info
        );
        assert_eq!(
            serde_json::to_value(initialize.capabilities).unwrap(),
            expected_capabilities
        );
    }
}

//! Native adapters for invoking extension implementations without WASM.
//!
//! [`NativeExtension`] makes a single extension instance available through
//! typed frontend/backend/workspace handles and through the Morphir Extension
//! Protocol.

use crate::protocol::{ExtensionRequest, ExtensionResponse};
use crate::{
    __dispatch_backend, __dispatch_frontend, __dispatch_workspace, __extension_info, Backend,
    BackendCapability, CompileRequest, CompileResult, Extension, ExtensionCapabilities,
    ExtensionError, ExtensionInfo, ExtensionType, Frontend, FrontendCapability, GenerateRequest,
    GenerateResult, NativeRoleDispatch, Result, Workspace, WorkspaceCapability,
    dispatch_request_with_roles, erase_dispatch,
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

/// A typed native workspace endpoint.
pub trait NativeWorkspace: Send + Sync {
    /// Discover a workspace without serializing through the protocol.
    fn discover(
        &self,
        request: morphir_workspace::DiscoveryRequest,
    ) -> Result<morphir_workspace::DiscoveryResponse>;
}

/// A native Morphir Extension Protocol endpoint.
pub trait NativeProtocol: Send + Sync {
    /// Handle one JSON-RPC extension request.
    fn handle(&self, request: ExtensionRequest) -> ExtensionResponse;
}

/// A registered frontend role: its advertised capability, its typed handle and
/// its protocol dispatcher, bound together so they cannot disagree.
#[derive(Clone)]
struct FrontendRole {
    capability: FrontendCapability,
    handle: Arc<dyn NativeFrontend>,
    dispatch: NativeRoleDispatch,
}

/// A registered backend role. See [`FrontendRole`].
#[derive(Clone)]
struct BackendRole {
    capability: BackendCapability,
    handle: Arc<dyn NativeBackend>,
    dispatch: NativeRoleDispatch,
}

/// A registered workspace role. See [`FrontendRole`].
#[derive(Clone)]
struct WorkspaceRole {
    capability: WorkspaceCapability,
    handle: Arc<dyn NativeWorkspace>,
    dispatch: NativeRoleDispatch,
}

/// The roles of a constructed extension. A completed [`NativeExtension`] always
/// has at least one; this type alone does not enforce that, so it never
/// derives `Default` — that state is one a finished extension never has.
#[derive(Clone)]
struct NativeRoles {
    frontend: Option<FrontendRole>,
    backend: Option<BackendRole>,
    workspace: Option<WorkspaceRole>,
}

impl NativeRoles {
    /// Protocol dispatchers projected from the registered roles, frontend
    /// before backend before workspace, consumed by `NativeExtensionBuilder::finish`
    /// when it builds the protocol handle.
    fn dispatchers(&self) -> Vec<NativeRoleDispatch> {
        let mut dispatchers = Vec::new();
        if let Some(frontend) = &self.frontend {
            dispatchers.push(frontend.dispatch.clone());
        }
        if let Some(backend) = &self.backend {
            dispatchers.push(backend.dispatch.clone());
        }
        if let Some(workspace) = &self.workspace {
            dispatchers.push(workspace.dispatch.clone());
        }
        dispatchers
    }
}

/// Capability values that belong to no single role.
#[derive(Clone)]
struct CommonCapabilities {
    streaming: bool,
    incremental: bool,
    cancellation: bool,
    progress: bool,
    extra: std::collections::HashMap<String, serde_json::Value>,
}

/// Project the wire-shaped [`ExtensionCapabilities`] from a constructed
/// extension's roles and common flags.
///
/// This is the single authority for that projection: [`NativeExtension::capabilities`]
/// and [`NativeExtensionBuilder::finish`] both call it, rather than each
/// recomputing the same shape, so the value an extension answers `morphir.capabilities`
/// with can never drift from what `capabilities()` returns.
fn project_capabilities(roles: &NativeRoles, common: &CommonCapabilities) -> ExtensionCapabilities {
    ExtensionCapabilities {
        frontend: roles.frontend.as_ref().map(|role| role.capability.clone()),
        backend: roles.backend.as_ref().map(|role| role.capability.clone()),
        workspace: roles.workspace.as_ref().map(|role| role.capability.clone()),
        streaming: common.streaming,
        incremental: common.incremental,
        cancellation: common.cancellation,
        progress: common.progress,
        extra: common.extra.clone(),
    }
}

/// An extension implementation exposed through native typed and protocol APIs.
#[derive(Clone)]
pub struct NativeExtension {
    info: ExtensionInfo,
    roles: NativeRoles,
    common: CommonCapabilities,
    protocol: Arc<dyn NativeProtocol>,
}

/// Fixtures used only by this crate's doctests. Not part of the public API.
#[doc(hidden)]
pub mod doc_fixtures {
    use crate::{
        CompileRequest, CompileResult, Extension, ExtensionCapabilities, ExtensionInfo, Frontend,
        FrontendCapability, Result,
    };

    /// A minimal frontend extension used to demonstrate [`super::NativeExtension::builder`].
    #[derive(Default)]
    pub struct DocFrontend;

    impl Extension for DocFrontend {
        fn info() -> ExtensionInfo {
            ExtensionInfo {
                id: "doc-frontend".into(),
                name: "Doc frontend".into(),
                version: "1.0.0".into(),
                ..ExtensionInfo::default()
            }
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

    impl Frontend for DocFrontend {
        fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
            Ok(CompileResult {
                success: true,
                ir_version: None,
                ir: None,
                diagnostics: vec![],
                modules: request.package.exposed_modules.unwrap_or_default(),
                module_results: vec![],
                context_digest: None,
            })
        }

        fn supported_languages() -> Vec<String> {
            vec![]
        }

        fn file_extensions() -> Vec<String> {
            vec![]
        }
    }
}

/// Builder state: no role has been registered yet.
pub struct Empty;
/// Builder state: at least one role has been registered.
pub struct NonEmpty;

struct PendingFrontend {
    handle: Arc<dyn NativeFrontend>,
    dispatch: NativeRoleDispatch,
}

struct PendingBackend {
    handle: Arc<dyn NativeBackend>,
    dispatch: NativeRoleDispatch,
}

struct PendingWorkspace {
    handle: Arc<dyn NativeWorkspace>,
    dispatch: NativeRoleDispatch,
}

/// Accumulates an extension's roles. Consuming, so a stale builder cannot be
/// mistaken for the finished registration.
pub struct NativeExtensionBuilder<E, State> {
    extension: Arc<E>,
    frontend: Option<PendingFrontend>,
    backend: Option<PendingBackend>,
    workspace: Option<PendingWorkspace>,
    state: std::marker::PhantomData<State>,
}

impl NativeExtension {
    /// Start building an extension from its roles.
    ///
    /// Adding a role is how the builder becomes finishable:
    ///
    /// ```
    /// # use morphir_extension_sdk::NativeExtension;
    /// # use morphir_extension_sdk::native::doc_fixtures::DocFrontend;
    /// let extension = NativeExtension::builder(DocFrontend).with_frontend().finish();
    /// assert!(extension.is_ok());
    /// ```
    ///
    /// An empty builder is legal to hold:
    ///
    /// ```
    /// # use morphir_extension_sdk::NativeExtension;
    /// # use morphir_extension_sdk::native::doc_fixtures::DocFrontend;
    /// let _builder = NativeExtension::builder(DocFrontend);
    /// ```
    ///
    /// But it cannot be finished — `finish` does not exist until a role is added:
    ///
    /// ```compile_fail
    /// # use morphir_extension_sdk::NativeExtension;
    /// # use morphir_extension_sdk::native::doc_fixtures::DocFrontend;
    /// let extension = NativeExtension::builder(DocFrontend).finish();
    /// ```
    pub fn builder<E>(extension: E) -> NativeExtensionBuilder<E, Empty>
    where
        E: Extension + Send + Sync + 'static,
    {
        NativeExtensionBuilder {
            extension: Arc::new(extension),
            frontend: None,
            backend: None,
            workspace: None,
            state: std::marker::PhantomData,
        }
    }

    /// Expose an extension that provides both frontend and backend capabilities.
    pub fn frontend_backend<E>(extension: E) -> Result<Self>
    where
        E: Extension + Frontend + Backend + Send + Sync + 'static,
    {
        Self::builder(extension)
            .with_frontend()
            .with_backend()
            .finish()
    }

    /// Expose an extension that provides only a frontend capability.
    pub fn frontend_only<E>(extension: E) -> Result<Self>
    where
        E: Extension + Frontend + Send + Sync + 'static,
    {
        Self::builder(extension).with_frontend().finish()
    }

    /// Expose an extension that provides only a backend capability.
    pub fn backend_only<E>(extension: E) -> Result<Self>
    where
        E: Extension + Backend + Send + Sync + 'static,
    {
        Self::builder(extension).with_backend().finish()
    }

    /// Return the extension metadata, including the declared native handles.
    pub fn info(&self) -> &ExtensionInfo {
        &self.info
    }

    /// Return the extension's advertised capabilities, projected from the
    /// registered roles. This clones the frontend, backend, and workspace
    /// capability records plus the `extra` map on every call, so prefer
    /// calling it once and reusing the result over a hot path.
    pub fn capabilities(&self) -> ExtensionCapabilities {
        project_capabilities(&self.roles, &self.common)
    }

    /// Return the direct frontend handle when the provider exposes one.
    pub fn frontend(&self) -> Option<&dyn NativeFrontend> {
        self.roles
            .frontend
            .as_ref()
            .map(|role| role.handle.as_ref())
    }

    /// Return the direct backend handle when the provider exposes one.
    pub fn backend(&self) -> Option<&dyn NativeBackend> {
        self.roles.backend.as_ref().map(|role| role.handle.as_ref())
    }

    /// Return the direct workspace handle when the provider exposes one.
    pub fn workspace(&self) -> Option<&dyn NativeWorkspace> {
        self.roles
            .workspace
            .as_ref()
            .map(|role| role.handle.as_ref())
    }

    /// Return the protocol endpoint.
    pub fn protocol(&self) -> &dyn NativeProtocol {
        self.protocol.as_ref()
    }
}

impl<E, State> NativeExtensionBuilder<E, State>
where
    E: Extension + Send + Sync + 'static,
{
    /// Register this extension's frontend role. Calling it twice replaces the
    /// earlier registration.
    pub fn with_frontend(self) -> NativeExtensionBuilder<E, NonEmpty>
    where
        E: Frontend,
    {
        let handle = Arc::new(FrontendHandle {
            extension: Arc::clone(&self.extension),
        }) as Arc<dyn NativeFrontend>;
        let dispatch = erase_dispatch(Arc::clone(&self.extension), __dispatch_frontend::<E>);
        NativeExtensionBuilder {
            extension: self.extension,
            frontend: Some(PendingFrontend { handle, dispatch }),
            backend: self.backend,
            workspace: self.workspace,
            state: std::marker::PhantomData,
        }
    }

    /// Register this extension's backend role. Calling it twice replaces the
    /// earlier registration.
    pub fn with_backend(self) -> NativeExtensionBuilder<E, NonEmpty>
    where
        E: Backend,
    {
        let handle = Arc::new(BackendHandle {
            extension: Arc::clone(&self.extension),
        }) as Arc<dyn NativeBackend>;
        let dispatch = erase_dispatch(Arc::clone(&self.extension), __dispatch_backend::<E>);
        NativeExtensionBuilder {
            extension: self.extension,
            frontend: self.frontend,
            backend: Some(PendingBackend { handle, dispatch }),
            workspace: self.workspace,
            state: std::marker::PhantomData,
        }
    }

    /// Register this extension's workspace role. Calling it twice replaces the
    /// earlier registration.
    pub fn with_workspace(self) -> NativeExtensionBuilder<E, NonEmpty>
    where
        E: Workspace,
    {
        let handle = Arc::new(WorkspaceHandle {
            extension: Arc::clone(&self.extension),
        }) as Arc<dyn NativeWorkspace>;
        let dispatch = erase_dispatch(Arc::clone(&self.extension), __dispatch_workspace::<E>);
        NativeExtensionBuilder {
            extension: self.extension,
            frontend: self.frontend,
            backend: self.backend,
            workspace: Some(PendingWorkspace { handle, dispatch }),
            state: std::marker::PhantomData,
        }
    }

    /// Extension types projected from the pending registrations, frontend
    /// before backend before workspace. This is needed before a [`NativeRoles`]
    /// exists — it feeds `__extension_info::<E>()` and `validate_capabilities`,
    /// and roles are only materialized after validation passes.
    fn declared_types(&self) -> Vec<ExtensionType> {
        let mut types = Vec::new();
        if self.frontend.is_some() {
            types.push(ExtensionType::Frontend);
        }
        if self.backend.is_some() {
            types.push(ExtensionType::Backend);
        }
        if self.workspace.is_some() {
            types.push(ExtensionType::Workspace);
        }
        types
    }
}

impl<E> NativeExtensionBuilder<E, NonEmpty>
where
    E: Extension + Send + Sync + 'static,
{
    /// Validate the authored capabilities against the registered roles and
    /// materialize the finished extension.
    pub fn finish(self) -> Result<NativeExtension> {
        let declared_types = self.declared_types();

        let info = __extension_info::<E>(&declared_types);
        let capabilities = E::capabilities();
        validate_capabilities(&info, &capabilities, &declared_types)?;
        validate_protocol_metadata(&info, &capabilities)?;

        // Validation passed against the authored aggregate above; only now is
        // it safe to pair each pending registration with its capability.
        let roles = NativeRoles {
            frontend: self.frontend.map(|pending| FrontendRole {
                capability: capabilities
                    .frontend
                    .clone()
                    .expect("validate_capabilities confirmed a frontend capability"),
                handle: pending.handle,
                dispatch: pending.dispatch,
            }),
            backend: self.backend.map(|pending| BackendRole {
                capability: capabilities
                    .backend
                    .clone()
                    .expect("validate_capabilities confirmed a backend capability"),
                handle: pending.handle,
                dispatch: pending.dispatch,
            }),
            workspace: self.workspace.map(|pending| WorkspaceRole {
                capability: capabilities
                    .workspace
                    .clone()
                    .expect("validate_capabilities confirmed a workspace capability"),
                handle: pending.handle,
                dispatch: pending.dispatch,
            }),
        };

        let common = CommonCapabilities {
            streaming: capabilities.streaming,
            incremental: capabilities.incremental,
            cancellation: capabilities.cancellation,
            progress: capabilities.progress,
            extra: capabilities.extra.clone(),
        };

        // Feed the protocol handle the same projection `capabilities()` computes,
        // not the authored aggregate validated above — see `project_capabilities`.
        let protocol = Arc::new(ProtocolHandle {
            dispatchers: roles.dispatchers(),
            info: info.clone(),
            capabilities: project_capabilities(&roles, &common),
        });

        Ok(NativeExtension {
            info,
            roles,
            common,
            protocol,
        })
    }
}

fn validate_capabilities(
    info: &ExtensionInfo,
    capabilities: &ExtensionCapabilities,
    declared_types: &[ExtensionType],
) -> Result<()> {
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
        (true, Some(backend)) if backend.generate => {}
        (true, _) => {
            return Err(ExtensionError::UnsupportedCapability {
                extension: info.id.clone(),
                capability: "backend.generate".into(),
            });
        }
        (false, None) => {}
        (false, Some(_)) => {
            return Err(ExtensionError::InitFailed(
                "extension advertises backend without a native backend handle".into(),
            ));
        }
    }

    let has_workspace_handle = declared_types.contains(&ExtensionType::Workspace);
    match (has_workspace_handle, capabilities.workspace.as_ref()) {
        (true, Some(workspace)) if workspace.discover => {}
        (true, _) => {
            return Err(ExtensionError::UnsupportedCapability {
                extension: info.id.clone(),
                capability: "workspace.discover".into(),
            });
        }
        (false, None) => {}
        (false, Some(_)) => {
            return Err(ExtensionError::InitFailed(
                "extension advertises workspace without a native workspace handle".into(),
            ));
        }
    }

    Ok(())
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
        // This handle lets a host call a native extension without going
        // through the protocol's `serde` boundary, so the legacy source-root
        // keys are not rejected by deserialization here. Reject them
        // explicitly to keep this path equivalent to the protocol path
        // rather than a quieter way around it.
        //
        // This stays strict while the wire boundary transitionally accepts the
        // legacy envelope (see `types::SourceEnvelope`). That leniency exists
        // for hosts that were released before `CompileRequest.sources` and can
        // only speak the old shape. An in-process caller has no such problem:
        // it builds a `CompileRequest` in Rust against this very crate, so it
        // already has `sources.root` and a legacy key in `options.extra` can
        // only be a mistake — and one that would leave the root stated twice.
        crate::types::reject_legacy_source_root_keys(&request.options.extra)
            .map_err(ExtensionError::InvalidParams)?;
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

struct WorkspaceHandle<E> {
    extension: Arc<E>,
}

impl<E> NativeWorkspace for WorkspaceHandle<E>
where
    E: Workspace + Send + Sync,
{
    fn discover(
        &self,
        request: morphir_workspace::DiscoveryRequest,
    ) -> Result<morphir_workspace::DiscoveryResponse> {
        self.extension.discover(request)
    }
}

struct ProtocolHandle {
    dispatchers: Vec<NativeRoleDispatch>,
    info: ExtensionInfo,
    capabilities: ExtensionCapabilities,
}

impl NativeProtocol for ProtocolHandle {
    fn handle(&self, request: ExtensionRequest) -> ExtensionResponse {
        dispatch_request_with_roles(&request, &self.dispatchers, &self.info, &self.capabilities)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BackendHandle, BackendRole, CommonCapabilities, FrontendHandle, FrontendRole, NativeRoles,
        NativeWorkspace, WorkspaceHandle, WorkspaceRole, project_capabilities,
    };
    use crate::ExtensionError;
    use crate::NativeExtension;
    use crate::protocol::{ExtensionRequest, methods};
    use crate::{
        Artifact, Backend, BackendCapability, CompileOptions, CompilePackage, CompileRequest,
        CompileResult, Extension, ExtensionCapabilities, ExtensionInfo, ExtensionType, Frontend,
        FrontendCapability, GenerateRequest, GenerateResult, LanguageCapability,
        NativeRoleDispatch, Result, SourceDocument, SourceSet, Workspace, WorkspaceCapability,
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
                ir: Some(serde_json::json!({ "typed": request.sources.documents[0].text })),
                diagnostics: vec![],
                modules: request.package.exposed_modules.unwrap_or_default(),
                module_results: vec![],
                context_digest: None,
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
                modules: request.package.exposed_modules.unwrap_or_default(),
                module_results: vec![],
                context_digest: None,
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
                modules: request.package.exposed_modules.unwrap_or_default(),
                module_results: vec![],
                context_digest: None,
            })
        }

        fn supported_languages() -> Vec<String> {
            vec![]
        }

        fn file_extensions() -> Vec<String> {
            vec![]
        }
    }

    fn a_workspace_discovery_request() -> morphir_workspace::DiscoveryRequest {
        use morphir_workspace::{FileEntry, FileTree, RelativePath};
        morphir_workspace::DiscoveryRequest {
            protocol_version: 1,
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

    /// A frontend implementing `Workspace` too, used to prove that
    /// registering the workspace role *before* `with_frontend` doesn't get
    /// silently dropped by `with_frontend`'s rebuilt builder literal. IR
    /// versions are non-empty so `frontend.compile` validation succeeds for
    /// an unrelated reason doesn't mask the thing under test.
    #[derive(Default)]
    struct FrontendAndWorkspace;

    impl Extension for FrontendAndWorkspace {
        fn info() -> ExtensionInfo {
            ExtensionInfo::default()
        }

        fn capabilities() -> ExtensionCapabilities {
            ExtensionCapabilities {
                frontend: Some(FrontendCapability {
                    ir_versions: vec!["3".into()],
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

    impl Frontend for FrontendAndWorkspace {
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

    impl Workspace for FrontendAndWorkspace {
        fn discover(
            &self,
            request: morphir_workspace::DiscoveryRequest,
        ) -> Result<morphir_workspace::DiscoveryResponse> {
            Ok(morphir_workspace::discover(request))
        }
    }

    /// A backend implementing `Workspace` too, the `with_backend` mirror of
    /// `FrontendAndWorkspace` — proves the workspace role registered before
    /// `with_backend` survives its rebuilt builder literal.
    #[derive(Default)]
    struct BackendAndWorkspace;

    impl Extension for BackendAndWorkspace {
        fn info() -> ExtensionInfo {
            ExtensionInfo::default()
        }

        fn capabilities() -> ExtensionCapabilities {
            ExtensionCapabilities {
                backend: Some(BackendCapability {
                    ir_versions: vec!["3".into()],
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

    impl Backend for BackendAndWorkspace {
        fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
            BackendOnly.generate(request)
        }

        fn target_languages() -> Vec<String> {
            vec![]
        }
    }

    impl Workspace for BackendAndWorkspace {
        fn discover(
            &self,
            request: morphir_workspace::DiscoveryRequest,
        ) -> Result<morphir_workspace::DiscoveryResponse> {
            Ok(morphir_workspace::discover(request))
        }
    }

    /// A minimal workspace-only extension: `capabilities()` advertises only a
    /// `WorkspaceCapability`, and `discover` delegates to
    /// `morphir_workspace::discover`. Used both hand-driven (to exercise
    /// `WorkspaceHandle`/`WorkspaceRole` directly, the same way
    /// `native_roles_dispatchers_are_frontend_before_backend` constructs a
    /// `NativeRoles` directly) and through the builder, to prove single-role
    /// workspace construction — unlike
    /// `FrontendWithWorkspace`/`BackendWithWorkspace`/`FrontendBackendWithWorkspace`,
    /// which advertise a workspace capability without implementing `Workspace`.
    #[derive(Default)]
    struct WorkspaceOnly;

    impl Extension for WorkspaceOnly {
        fn info() -> ExtensionInfo {
            ExtensionInfo::default()
        }

        fn capabilities() -> ExtensionCapabilities {
            ExtensionCapabilities {
                workspace: Some(WorkspaceCapability {
                    protocol_versions: vec![1],
                    discover: true,
                }),
                ..ExtensionCapabilities::default()
            }
        }
    }

    impl Workspace for WorkspaceOnly {
        fn discover(
            &self,
            request: morphir_workspace::DiscoveryRequest,
        ) -> Result<morphir_workspace::DiscoveryResponse> {
            Ok(morphir_workspace::discover(request))
        }
    }

    /// A workspace handle is registered, but the authored capabilities carry
    /// no workspace record at all — the `(true, None)` arm of the workspace
    /// reconciliation, mirroring how a registered frontend without an
    /// advertised `FrontendCapability` is rejected.
    #[derive(Default)]
    struct WorkspaceWithoutAdvertisedCapability;

    impl Extension for WorkspaceWithoutAdvertisedCapability {
        fn info() -> ExtensionInfo {
            ExtensionInfo::default()
        }

        fn capabilities() -> ExtensionCapabilities {
            ExtensionCapabilities::default()
        }
    }

    impl Workspace for WorkspaceWithoutAdvertisedCapability {
        fn discover(
            &self,
            request: morphir_workspace::DiscoveryRequest,
        ) -> Result<morphir_workspace::DiscoveryResponse> {
            Ok(morphir_workspace::discover(request))
        }
    }

    /// A workspace protocol version no real host advertises. Construction
    /// must still succeed: `validate_capabilities` only reconciles capability
    /// presence and `discover: true`, never `protocol_versions` — host
    /// compatibility is the daemon's concern at invocation time
    /// (`controller.rs:71-99`), not the extension's at construction time.
    #[derive(Default)]
    struct WorkspaceWithIncompatibleProtocolVersion;

    impl Extension for WorkspaceWithIncompatibleProtocolVersion {
        fn info() -> ExtensionInfo {
            ExtensionInfo::default()
        }

        fn capabilities() -> ExtensionCapabilities {
            ExtensionCapabilities {
                workspace: Some(WorkspaceCapability {
                    protocol_versions: vec![9999],
                    discover: true,
                }),
                ..ExtensionCapabilities::default()
            }
        }
    }

    impl Workspace for WorkspaceWithIncompatibleProtocolVersion {
        fn discover(
            &self,
            request: morphir_workspace::DiscoveryRequest,
        ) -> Result<morphir_workspace::DiscoveryResponse> {
            Ok(morphir_workspace::discover(request))
        }
    }

    /// A workspace-only extension that records every discovery request it
    /// receives, mirroring `RecordingExtension`'s `compile_requests`. Used by
    /// `direct_and_protocol_workspace_dispatch_are_equivalent` to prove that
    /// the direct handle and the protocol handle dispatch through the same
    /// registered instance rather than two independently constructed ones.
    #[derive(Default)]
    struct RecordingWorkspace {
        discovery_requests: Arc<Mutex<Vec<morphir_workspace::DiscoveryRequest>>>,
    }

    impl Extension for RecordingWorkspace {
        fn info() -> ExtensionInfo {
            ExtensionInfo::default()
        }

        fn capabilities() -> ExtensionCapabilities {
            ExtensionCapabilities {
                workspace: Some(WorkspaceCapability {
                    protocol_versions: vec![1],
                    discover: true,
                }),
                ..ExtensionCapabilities::default()
            }
        }
    }

    impl Workspace for RecordingWorkspace {
        fn discover(
            &self,
            request: morphir_workspace::DiscoveryRequest,
        ) -> Result<morphir_workspace::DiscoveryResponse> {
            self.discovery_requests
                .lock()
                .unwrap()
                .push(request.clone());
            Ok(morphir_workspace::discover(request))
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

    #[derive(Clone, Default)]
    struct FullyDecoratedExtension {
        compile_requests: Arc<Mutex<Vec<CompileRequest>>>,
    }

    impl Extension for FullyDecoratedExtension {
        fn info() -> ExtensionInfo {
            ExtensionInfo {
                id: "fully-decorated".into(),
                name: "Fully decorated extension".into(),
                version: "1.0.0".into(),
                ..ExtensionInfo::default()
            }
        }

        fn capabilities() -> ExtensionCapabilities {
            ExtensionCapabilities {
                frontend: Some(FrontendCapability {
                    languages: vec![LanguageCapability {
                        id: "decorated".into(),
                        file_extensions: vec![".decorated".into()],
                    }],
                    ir_versions: vec!["3".into()],
                    compile: true,
                    incremental: false,
                    fragments: false,
                }),
                backend: Some(BackendCapability {
                    targets: vec!["decorated".into()],
                    ir_versions: vec!["3".into()],
                    generate: true,
                }),
                workspace: None,
                streaming: true,
                incremental: true,
                cancellation: true,
                progress: true,
                extra: [(
                    "experimental".into(),
                    serde_json::json!({ "enabled": true }),
                )]
                .into_iter()
                .collect(),
            }
        }
    }

    impl Frontend for FullyDecoratedExtension {
        fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
            self.compile_requests.lock().unwrap().push(request.clone());
            FrontendOnly.compile(request)
        }

        fn supported_languages() -> Vec<String> {
            vec!["decorated".into()]
        }

        fn file_extensions() -> Vec<String> {
            vec![".decorated".into()]
        }
    }

    impl Backend for FullyDecoratedExtension {
        fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
            BackendOnly.generate(request)
        }

        fn target_languages() -> Vec<String> {
            vec!["decorated".into()]
        }
    }

    /// `NativeRoles::dispatchers` must order the frontend dispatcher before the
    /// backend one. A length check alone would pass for a reversed `Vec`, so
    /// each dispatcher answers a probe request with a distinct, identifiable
    /// payload and the response at each index is checked against it.
    #[test]
    fn native_roles_dispatchers_are_frontend_before_backend() {
        let frontend_marker: NativeRoleDispatch =
            Arc::new(|_request: &ExtensionRequest| Some(Ok(serde_json::json!("frontend"))));
        let backend_marker: NativeRoleDispatch =
            Arc::new(|_request: &ExtensionRequest| Some(Ok(serde_json::json!("backend"))));
        let roles = NativeRoles {
            frontend: Some(FrontendRole {
                capability: FrontendCapability::default(),
                handle: Arc::new(FrontendHandle {
                    extension: Arc::new(FrontendOnly),
                }),
                dispatch: frontend_marker,
            }),
            backend: Some(BackendRole {
                capability: BackendCapability::default(),
                handle: Arc::new(BackendHandle {
                    extension: Arc::new(BackendOnly),
                }),
                dispatch: backend_marker,
            }),
            workspace: None,
        };

        let dispatchers = roles.dispatchers();
        assert_eq!(dispatchers.len(), 2);
        let probe = ExtensionRequest::new("probe", serde_json::json!({}), 1).unwrap();
        let first = dispatchers[0](&probe)
            .expect("dispatcher at index 0 should answer")
            .expect("dispatcher at index 0 should succeed");
        assert_eq!(first, serde_json::json!("frontend"));
        let second = dispatchers[1](&probe)
            .expect("dispatcher at index 1 should answer")
            .expect("dispatcher at index 1 should succeed");
        assert_eq!(second, serde_json::json!("backend"));
    }

    #[test]
    fn native_roles_dispatchers_include_only_the_registered_role() {
        let no_op: NativeRoleDispatch = Arc::new(|_request: &ExtensionRequest| None);
        let roles = NativeRoles {
            frontend: None,
            backend: Some(BackendRole {
                capability: BackendCapability::default(),
                handle: Arc::new(BackendHandle {
                    extension: Arc::new(BackendOnly),
                }),
                dispatch: no_op,
            }),
            workspace: None,
        };

        assert_eq!(roles.dispatchers().len(), 1);
    }

    /// `NativeRoles::dispatchers` must order the workspace dispatcher last,
    /// after frontend and backend. Same distinct-payload technique as
    /// `native_roles_dispatchers_are_frontend_before_backend`, extended to all
    /// three roles.
    #[test]
    fn native_roles_dispatchers_put_workspace_last() {
        let frontend_marker: NativeRoleDispatch =
            Arc::new(|_request: &ExtensionRequest| Some(Ok(serde_json::json!("frontend"))));
        let backend_marker: NativeRoleDispatch =
            Arc::new(|_request: &ExtensionRequest| Some(Ok(serde_json::json!("backend"))));
        let workspace_marker: NativeRoleDispatch =
            Arc::new(|_request: &ExtensionRequest| Some(Ok(serde_json::json!("workspace"))));
        let roles = NativeRoles {
            frontend: Some(FrontendRole {
                capability: FrontendCapability::default(),
                handle: Arc::new(FrontendHandle {
                    extension: Arc::new(FrontendOnly),
                }),
                dispatch: frontend_marker,
            }),
            backend: Some(BackendRole {
                capability: BackendCapability::default(),
                handle: Arc::new(BackendHandle {
                    extension: Arc::new(BackendOnly),
                }),
                dispatch: backend_marker,
            }),
            workspace: Some(WorkspaceRole {
                capability: WorkspaceCapability::default(),
                handle: Arc::new(WorkspaceHandle {
                    extension: Arc::new(WorkspaceOnly),
                }),
                dispatch: workspace_marker,
            }),
        };

        let dispatchers = roles.dispatchers();
        assert_eq!(dispatchers.len(), 3);
        let probe = ExtensionRequest::new("probe", serde_json::json!({}), 1).unwrap();
        let third = dispatchers[2](&probe)
            .expect("dispatcher at index 2 should answer")
            .expect("dispatcher at index 2 should succeed");
        assert_eq!(third, serde_json::json!("workspace"));
    }

    /// `WorkspaceHandle` must forward `discover` to the wrapped extension
    /// unchanged, mirroring `FrontendHandle`/`BackendHandle`.
    #[test]
    fn workspace_handle_forwards_to_the_extension() {
        let handle = WorkspaceHandle {
            extension: Arc::new(WorkspaceOnly),
        };

        let request = a_workspace_discovery_request();
        let direct = WorkspaceOnly.discover(request.clone()).unwrap();
        let through_handle = handle.discover(request).unwrap();

        assert_eq!(
            serde_json::to_value(&direct).unwrap(),
            serde_json::to_value(&through_handle).unwrap(),
        );
    }

    /// `project_capabilities` must include a registered workspace role's
    /// capability. Before this fix the field was hard-coded to `None`, which
    /// was correct only because no workspace role could be constructed; this
    /// proves the projection is wired now that a role can carry one.
    #[test]
    fn project_capabilities_includes_a_registered_workspace_capability() {
        let capability = WorkspaceCapability {
            protocol_versions: vec![1],
            discover: true,
        };
        let roles = NativeRoles {
            frontend: None,
            backend: None,
            workspace: Some(WorkspaceRole {
                capability: capability.clone(),
                handle: Arc::new(WorkspaceHandle {
                    extension: Arc::new(WorkspaceOnly),
                }),
                dispatch: Arc::new(|_request: &ExtensionRequest| None),
            }),
        };
        let common = CommonCapabilities {
            streaming: false,
            incremental: false,
            cancellation: false,
            progress: false,
            extra: Default::default(),
        };

        let projected = project_capabilities(&roles, &common);

        assert_eq!(projected.workspace, Some(capability));
    }

    /// `NativeExtensionBuilder::declared_types` is the single place that
    /// projects pending registrations to `ExtensionType`s (see Important
    /// finding 2 in the native-role-registration review: the copy that used
    /// to live on `NativeRoles` had no production caller). Order matters —
    /// `info().types` mirrors it — so this asserts the full ordered `Vec`,
    /// not just its length.
    #[test]
    fn builder_declared_types_are_frontend_before_backend() {
        let builder = NativeExtension::builder(RecordingExtension::default())
            .with_frontend()
            .with_backend();

        assert_eq!(
            builder.declared_types(),
            [ExtensionType::Frontend, ExtensionType::Backend]
        );
    }

    #[test]
    fn builder_declared_types_reports_only_the_registered_role() {
        let builder = NativeExtension::builder(BackendOnly).with_backend();

        assert_eq!(builder.declared_types(), [ExtensionType::Backend]);
    }

    /// `with_frontend` documents that calling it twice replaces the earlier
    /// registration rather than accumulating both. `frontend` is a single
    /// `Option`, so a second call always overwrites — this test locks that
    /// observable behaviour in from `info().types` rather than leaving it as
    /// an untested doc comment.
    #[test]
    fn with_frontend_called_twice_replaces_the_earlier_registration() {
        let extension = NativeExtension::builder(FrontendOnly)
            .with_frontend()
            .with_frontend()
            .finish()
            .unwrap();

        assert_eq!(extension.info().types, [ExtensionType::Frontend]);
    }

    fn compile_request(source: &str) -> CompileRequest {
        CompileRequest {
            language_id: "recording".into(),
            sources: SourceSet {
                root: None,
                documents: vec![SourceDocument {
                    uri: "file:///workspace/Example.recording".into(),
                    language_id: "recording".into(),
                    version: 1,
                    text: source.into(),
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
                && recorded.sources.documents[0].text == request.sources.documents[0].text
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

    /// A workspace-bearing extension builds, and its capability survives the
    /// projection. Before this change `validate_capabilities` rejected it
    /// unconditionally, regardless of whether a handle existed.
    #[test]
    fn a_workspace_extension_builds_and_projects_its_capability() {
        let extension = NativeExtension::builder(WorkspaceOnly)
            .with_workspace()
            .finish()
            .expect("a workspace-only extension with a registered handle is valid");

        assert!(extension.workspace().is_some());
        assert_eq!(extension.info().types, [ExtensionType::Workspace]);
        assert_eq!(
            extension.capabilities().workspace,
            Some(WorkspaceCapability {
                protocol_versions: vec![1],
                discover: true,
            })
        );
    }

    /// A workspace role registered *before* `with_frontend` must survive
    /// `with_frontend`'s rebuilt `NativeExtensionBuilder` literal — the
    /// hazard the plan named explicitly: "a setter that drops it would
    /// silently discard a registered role." Every other workspace
    /// construction test in this module is workspace-only, so none of them
    /// puts `with_frontend`'s `workspace: self.workspace` line on a path
    /// where dropping it would be observable; this one does.
    ///
    /// The registration order is workspace-then-frontend, but
    /// `declared_types` always projects frontend before backend before
    /// workspace regardless of registration order — that ordering guarantee
    /// is the second thing this test pins down.
    #[test]
    fn workspace_registered_before_frontend_survives_with_frontend() {
        let extension = NativeExtension::builder(FrontendAndWorkspace)
            .with_workspace()
            .with_frontend()
            .finish()
            .unwrap();

        assert_eq!(
            extension.info().types,
            [ExtensionType::Frontend, ExtensionType::Workspace]
        );
        assert!(extension.frontend().is_some());
        assert!(extension.workspace().is_some());
        assert!(extension.capabilities().workspace.is_some());
    }

    /// The `with_backend` mirror of
    /// `workspace_registered_before_frontend_survives_with_frontend`: a
    /// workspace role registered before `with_backend` must survive its
    /// rebuilt builder literal too.
    #[test]
    fn workspace_registered_before_backend_survives_with_backend() {
        let extension = NativeExtension::builder(BackendAndWorkspace)
            .with_workspace()
            .with_backend()
            .finish()
            .unwrap();

        assert_eq!(
            extension.info().types,
            [ExtensionType::Backend, ExtensionType::Workspace]
        );
        assert!(extension.backend().is_some());
        assert!(extension.workspace().is_some());
        assert!(extension.capabilities().workspace.is_some());
    }

    /// Registering a workspace role still requires the advertised capability,
    /// exactly as a frontend without `FrontendCapability` is rejected.
    #[test]
    fn a_registered_workspace_without_an_advertised_capability_is_rejected() {
        let result = NativeExtension::builder(WorkspaceWithoutAdvertisedCapability)
            .with_workspace()
            .finish();

        assert!(matches!(
            result,
            Err(ExtensionError::UnsupportedCapability { capability, .. })
                if capability == "workspace.discover"
        ));
    }

    /// A workspace protocol version this host cannot speak is still a
    /// construction-time non-issue: `validate_capabilities` reconciles only
    /// capability presence and `discover: true`. Host compatibility is the
    /// daemon's concern at invocation time (`controller.rs:71-99`), so an
    /// extension whose workspace protocol is incompatible with this host must
    /// still construct as a usable extension.
    #[test]
    fn a_workspace_extension_with_an_incompatible_protocol_version_still_constructs() {
        let result = NativeExtension::builder(WorkspaceWithIncompatibleProtocolVersion)
            .with_workspace()
            .finish();

        assert!(result.is_ok());
    }

    /// Mirrors `direct_and_protocol_frontend_dispatch_are_equivalent`: the same
    /// `DiscoveryRequest`, sent through `workspace().unwrap().discover(..)` and
    /// through `protocol().handle(WORKSPACE_DISCOVER)`, must produce the same
    /// response from one shared provider instance. The recorded request count
    /// (2, one per call) is what proves "shared" rather than "two providers
    /// that happen to answer identically."
    #[test]
    fn direct_and_protocol_workspace_dispatch_are_equivalent() {
        let extension = RecordingWorkspace::default();
        let recorded_requests = extension.discovery_requests.clone();
        let provider = NativeExtension::builder(extension)
            .with_workspace()
            .finish()
            .unwrap();
        let request = a_workspace_discovery_request();

        let direct = provider
            .workspace()
            .unwrap()
            .discover(request.clone())
            .unwrap();
        let rpc = ExtensionRequest::new(methods::WORKSPACE_DISCOVER, request.clone(), 7).unwrap();
        let protocol = provider.protocol().handle(rpc);
        let through_mep: morphir_workspace::DiscoveryResponse =
            serde_json::from_value(protocol.result.unwrap()).unwrap();

        let direct_result = serde_json::to_value(&direct).unwrap();
        let protocol_result = serde_json::to_value(&through_mep).unwrap();
        assert_eq!(direct_result, protocol_result);
        let recorded_requests = recorded_requests.lock().unwrap();
        assert_eq!(recorded_requests.len(), 2);
        assert!(recorded_requests.iter().all(|recorded| {
            recorded.protocol_version == request.protocol_version
                && recorded.development_root == request.development_root
        }));
        assert_eq!(provider.info().types, [ExtensionType::Workspace]);
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

    /// An unknown method must produce the same JSON-RPC method-not-found error
    /// after dispatch is erased as before it.
    #[test]
    fn unknown_method_is_method_not_found_through_role_dispatch() {
        let native = NativeExtension::frontend_only(FrontendOnly).expect("FrontendOnly is valid");
        let response = native.protocol().handle(
            ExtensionRequest::new("morphir.not.a.method", serde_json::json!({}), 7)
                .expect("request is well formed"),
        );
        let error = response.error.expect("unknown method must be an error");
        assert_eq!(error.code, -32601);
        assert!(
            error.message.contains("morphir.not.a.method"),
            "the error should name the method, got: {}",
            error.message
        );
    }

    /// Every control method must keep answering from the construction snapshots
    /// after the native path stops sharing the guest's dispatcher.
    ///
    /// `morphir.initialize` needs a well-formed `InitializeParams` payload (unlike
    /// the other control methods, which ignore `params`), so it gets one here
    /// instead of the `{}` the other four use.
    #[test]
    fn every_control_method_answers_through_the_native_adapter() {
        let native = NativeExtension::frontend_only(FrontendOnly).expect("FrontendOnly is valid");
        let initialize_params = serde_json::to_value(crate::protocol::InitializeParams {
            protocol_versions: vec![crate::protocol::MEP_VERSION.into()],
            host: crate::protocol::PeerInfo {
                name: "test-host".into(),
                version: "1.0.0".into(),
            },
        })
        .expect("initialize params should serialize");
        for (id, (method, params)) in [
            (methods::INITIALIZE, initialize_params),
            (methods::PING, serde_json::json!({})),
            (methods::INFO, serde_json::json!({})),
            (methods::CAPABILITIES, serde_json::json!({})),
            (methods::SHUTDOWN, serde_json::json!({})),
        ]
        .into_iter()
        .enumerate()
        {
            let response = native.protocol().handle(
                ExtensionRequest::new(method, params, id as u64 + 1)
                    .expect("request is well formed"),
            );
            assert!(
                response.error.is_none(),
                "{method} must succeed, got {:?}",
                response.error
            );
        }
    }

    /// The projected capabilities must equal what the author declared, field for
    /// field. `capabilities()` stops being a stored copy and becomes a projection
    /// of the role records, so this proves the projection is lossless — including
    /// the cross-cutting flags and `extra`, which no role owns.
    #[test]
    fn projected_capabilities_round_trip_the_authored_ones() {
        let authored = FullyDecoratedExtension::capabilities();
        let native = NativeExtension::frontend_backend(FullyDecoratedExtension::default())
            .expect("frontend+backend extension is valid");
        assert_eq!(native.capabilities(), authored);
        assert_eq!(
            serde_json::to_value(native.capabilities()).unwrap(),
            serde_json::to_value(&authored).unwrap(),
            "the projection must also serialize identically",
        );
    }
}

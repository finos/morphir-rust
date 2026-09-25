//! Test doubles for clients of the host library.

use crate::channel::{Channel, ChannelError, Outgoing};
use crate::{
    BasicChecks, CapabilityMetadataScope, ChannelCause, ChannelState, GuestConnection, GuestSource,
    HostError, InvocationMode, InvocationPolicy, JsonRpcConnection, ProviderOrigin,
};
use async_trait::async_trait;
use morphir_extension_sdk::protocol::{
    ExtensionResponse, InitializeResult, SUPPORTED_MEP_VERSIONS,
};
use morphir_extension_sdk::{
    ExtensionCapabilities, ExtensionInfo, ExtensionType, FrontendCapability,
};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// What a [`MemoryChannel`] was asked to do, shared with the test.
#[derive(Debug, Clone, Default)]
pub struct SentLog {
    messages: Arc<Mutex<Vec<Outgoing>>>,
    closes: Arc<Mutex<u32>>,
    aborts: Arc<Mutex<u32>>,
}

impl SentLog {
    /// The method of every message sent, in order.
    pub fn methods(&self) -> Vec<String> {
        self.messages
            .lock()
            .expect("the log is never poisoned")
            .iter()
            .map(|message| match message {
                Outgoing::Request(request) => request.method.clone(),
                Outgoing::Notification(notification) => notification.method.clone(),
            })
            .collect()
    }

    /// The id of every request sent, in order. Notifications are skipped.
    pub fn ids(&self) -> Vec<u64> {
        self.messages
            .lock()
            .expect("the log is never poisoned")
            .iter()
            .filter_map(|message| match message {
                Outgoing::Request(request) => Some(request.id),
                Outgoing::Notification(_) => None,
            })
            .collect()
    }

    /// How many times the channel was closed.
    pub fn closes(&self) -> u32 {
        *self.closes.lock().expect("the log is never poisoned")
    }

    /// How many times the channel was aborted.
    pub fn aborts(&self) -> u32 {
        *self.aborts.lock().expect("the log is never poisoned")
    }
}

/// A channel that answers from a script, in order.
#[derive(Debug, Default)]
pub struct MemoryChannel {
    answers: VecDeque<Result<ExtensionResponse, ChannelError>>,
    log: SentLog,
}

impl MemoryChannel {
    /// A channel with no answers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Answer the next request with `response`.
    pub fn respond(mut self, response: ExtensionResponse) -> Self {
        self.answers.push_back(Ok(response));
        self
    }

    /// Fail the next receive with `error`.
    pub fn fail(mut self, error: ChannelError) -> Self {
        self.answers.push_back(Err(error));
        self
    }

    /// A handle on what the channel receives from the host.
    pub fn log(&self) -> SentLog {
        self.log.clone()
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Channel for MemoryChannel {
    async fn send(&mut self, message: Outgoing) -> Result<(), ChannelError> {
        self.log
            .messages
            .lock()
            .expect("the log is never poisoned")
            .push(message);
        Ok(())
    }

    async fn receive(&mut self) -> Result<ExtensionResponse, ChannelError> {
        self.answers.pop_front().unwrap_or_else(|| {
            Err(ChannelError {
                message: "MemoryChannel has no scripted answer".into(),
                state: ChannelState::Stopped,
                cause: ChannelCause::Transport,
            })
        })
    }

    async fn close(&mut self) -> Result<ChannelState, ChannelError> {
        *self.log.closes.lock().expect("the log is never poisoned") += 1;
        Ok(ChannelState::Stopped)
    }

    async fn abort(&mut self) -> Result<ChannelState, ChannelError> {
        *self.log.aborts.lock().expect("the log is never poisoned") += 1;
        Ok(ChannelState::Stopped)
    }
}

/// An `initialize` answer from a frontend guest that enables compile.
pub fn frontend_initialize_result(id: &str) -> InitializeResult {
    InitializeResult {
        protocol_version: SUPPORTED_MEP_VERSIONS[0].to_string(),
        extension: ExtensionInfo {
            id: id.into(),
            name: id.into(),
            types: vec![ExtensionType::Frontend],
            ..ExtensionInfo::default()
        },
        capabilities: ExtensionCapabilities {
            frontend: Some(FrontendCapability {
                compile: true,
                ..FrontendCapability::default()
            }),
            ..ExtensionCapabilities::default()
        },
    }
}

/// How a [`FakeSource`] picks its invocation mode.
#[derive(Debug, Clone, Copy)]
enum FakeModes {
    /// Like a built-in: direct under `PreferDirect`, native MEP otherwise.
    Native,
    /// The same mode under every policy, like an installed runtime.
    Fixed(InvocationMode),
    /// One mode under each policy.
    PerPolicy {
        prefer_direct: InvocationMode,
        protocol_only: InvocationMode,
    },
}

/// A [`GuestSource`] made of metadata, for registry tests.
///
/// `connect` returns a [`JsonRpcConnection`] over a new [`MemoryChannel`]
/// that answers with the responses given to [`FakeSource::respond`], and
/// checks that the guest names itself with the source's ID.
#[derive(Debug, Clone)]
pub struct FakeSource {
    info: ExtensionInfo,
    capabilities: ExtensionCapabilities,
    origin: ProviderOrigin,
    scope: CapabilityMetadataScope,
    modes: FakeModes,
    incarnation: Option<String>,
    answers: Vec<ExtensionResponse>,
}

impl FakeSource {
    /// A source that looks like a built-in: origin `Builtin`, complete
    /// metadata, `NativeDirect` under `PreferDirect` and `NativeMep` under
    /// `ProtocolOnly`.
    pub fn builtin(info: ExtensionInfo, capabilities: ExtensionCapabilities) -> Self {
        Self {
            info,
            capabilities,
            origin: ProviderOrigin::Builtin,
            scope: CapabilityMetadataScope::Complete,
            modes: FakeModes::Native,
            incarnation: None,
            answers: Vec::new(),
        }
    }

    /// A source that looks like an installed provider: origin `Installed`,
    /// persisted frontend and backend metadata, and `mode` under every policy.
    pub fn installed(
        info: ExtensionInfo,
        capabilities: ExtensionCapabilities,
        mode: InvocationMode,
    ) -> Self {
        Self {
            info,
            capabilities,
            origin: ProviderOrigin::Installed,
            scope: CapabilityMetadataScope::PersistedFrontendBackend,
            modes: FakeModes::Fixed(mode),
            incarnation: None,
            answers: Vec::new(),
        }
    }

    /// Report `origin` instead.
    pub fn with_origin(mut self, origin: ProviderOrigin) -> Self {
        self.origin = origin;
        self
    }

    /// Report `scope` instead.
    pub fn with_scope(mut self, scope: CapabilityMetadataScope) -> Self {
        self.scope = scope;
        self
    }

    /// Use `mode` under every policy.
    pub fn with_mode(mut self, mode: InvocationMode) -> Self {
        self.modes = FakeModes::Fixed(mode);
        self
    }

    /// Use `prefer_direct` under [`InvocationPolicy::PreferDirect`] and
    /// `protocol_only` under [`InvocationPolicy::ProtocolOnly`].
    pub fn with_modes(
        mut self,
        prefer_direct: InvocationMode,
        protocol_only: InvocationMode,
    ) -> Self {
        self.modes = FakeModes::PerPolicy {
            prefer_direct,
            protocol_only,
        };
        self
    }

    /// Report `incarnation` from [`GuestSource::incarnation`], like two
    /// builds registered under one id and version.
    pub fn with_incarnation(mut self, incarnation: impl Into<String>) -> Self {
        self.incarnation = Some(incarnation.into());
        self
    }

    /// Answer the next request on every connection with `response`.
    pub fn respond(mut self, response: ExtensionResponse) -> Self {
        self.answers.push(response);
        self
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl GuestSource for FakeSource {
    fn info(&self) -> &ExtensionInfo {
        &self.info
    }

    fn capabilities(&self) -> &ExtensionCapabilities {
        &self.capabilities
    }

    fn origin(&self) -> ProviderOrigin {
        self.origin
    }

    fn capability_metadata_scope(&self) -> CapabilityMetadataScope {
        self.scope
    }

    fn invocation_mode(&self, policy: InvocationPolicy) -> InvocationMode {
        match (self.modes, policy) {
            (FakeModes::Native, InvocationPolicy::PreferDirect) => InvocationMode::NativeDirect,
            (FakeModes::Native, InvocationPolicy::ProtocolOnly) => InvocationMode::NativeMep,
            (FakeModes::Fixed(mode), _) => mode,
            (FakeModes::PerPolicy { prefer_direct, .. }, InvocationPolicy::PreferDirect) => {
                prefer_direct
            }
            (FakeModes::PerPolicy { protocol_only, .. }, InvocationPolicy::ProtocolOnly) => {
                protocol_only
            }
        }
    }

    fn incarnation(&self) -> Option<&str> {
        self.incarnation.as_deref()
    }

    async fn connect(&self, _workspace: &Path) -> Result<Box<dyn GuestConnection>, HostError> {
        let channel = self
            .answers
            .iter()
            .cloned()
            .fold(MemoryChannel::new(), MemoryChannel::respond);
        Ok(Box::new(JsonRpcConnection::new(
            channel,
            BasicChecks::new(self.info.id.clone()),
        )))
    }
}

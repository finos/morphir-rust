//! Test doubles for clients of the host library.

use crate::ChannelState;
use crate::channel::{Channel, ChannelError, Outgoing};
use async_trait::async_trait;
use morphir_extension_sdk::protocol::{
    ExtensionResponse, InitializeResult, SUPPORTED_MEP_VERSIONS,
};
use morphir_extension_sdk::{
    ExtensionCapabilities, ExtensionInfo, ExtensionType, FrontendCapability,
};
use std::collections::VecDeque;
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

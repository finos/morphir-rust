//! Runtime-erased compatibility API for existing session callers.

use crate::Result;
use crate::extensions::protocol::{InitializeParams, InitializeResult};
use async_trait::async_trait;

/// Observable lifecycle state for the compatibility session interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionSessionState {
    /// The extension has loaded but has not negotiated MEP.
    Starting,
    /// The extension has negotiated MEP and accepts operations.
    Ready,
    /// The extension completed shutdown.
    Stopped,
}

/// Compatibility interface for callers that erase typestate at runtime.
#[async_trait]
pub trait ExtensionSession {
    /// Return the current runtime-erased state.
    fn state(&self) -> ExtensionSessionState;
    /// Negotiate a protocol version and capabilities.
    async fn initialize(&mut self, params: InitializeParams) -> Result<InitializeResult>;
    /// Invoke one operation with JSON values.
    async fn invoke(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value>;
    /// Complete the session lifecycle.
    async fn shutdown(&mut self) -> Result<()>;
}

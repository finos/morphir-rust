//! A negotiated connection to one guest.

use crate::{HostError, MaybeSend, Negotiated};
use async_trait::async_trait;
use morphir_extension_sdk::protocol::InitializeParams;

/// How a call ended when it did not succeed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CallError {
    /// The guest or the host refused the call. The session is still ready.
    #[error(transparent)]
    Rejected(HostError),
    /// The session broke, and the connection closed its channel.
    #[error(transparent)]
    Failed(HostError),
    /// No guest could be opened for this attempt. The error is the one
    /// opening the guest or its session reported.
    ///
    /// When [`crate::Pool::call`] reports it while replacing a broken
    /// session, an earlier attempt of the same call may already have reached
    /// the guest, so this does not mean the call never ran.
    ///
    /// Only [`crate::Pool::call`] reports this: a [`GuestConnection`] or a
    /// [`crate::Session`] is already open when it is called.
    #[error(transparent)]
    Open(HostError),
}

/// One guest behind one binding of MEP.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait GuestConnection: MaybeSend {
    /// Run the MEP handshake.
    async fn open(&mut self, params: InitializeParams) -> Result<Negotiated, HostError>;

    /// Invoke one non-lifecycle method.
    async fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, CallError>;

    /// Complete MEP shutdown and release the guest.
    async fn close(&mut self) -> Result<(), HostError>;
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<G: GuestConnection + ?Sized> GuestConnection for Box<G> {
    async fn open(&mut self, params: InitializeParams) -> Result<Negotiated, HostError> {
        (**self).open(params).await
    }

    async fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, CallError> {
        (**self).call(method, params).await
    }

    async fn close(&mut self) -> Result<(), HostError> {
        (**self).close().await
    }
}

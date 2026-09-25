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
    /// The guest answered, but its result did not decode or failed the
    /// host's checks. The session was closed in order.
    ///
    /// [`crate::Session::call`] reports this for a result that does not
    /// decode. A [`GuestConnection`] that checks results, such as
    /// `morphir_host_native::CheckedConnection`, reports it for a result
    /// that fails a check.
    #[error(transparent)]
    Invalid(HostError),
    /// Only [`crate::Pool::call`]: the `open` function failed, so no guest
    /// was reached. The error is the one `open` reported.
    ///
    /// When the pool reports it while replacing a broken session, an earlier
    /// attempt of the same call may already have reached a guest, so this
    /// does not mean the call never ran.
    #[error(transparent)]
    Connect(HostError),
    /// Only [`crate::Pool::call`]: the guest started but the MEP handshake
    /// failed. The error is the one the handshake reported.
    ///
    /// When the pool reports it while replacing a broken session, an earlier
    /// attempt of the same call may already have reached a guest, so this
    /// does not mean the call never ran.
    #[error(transparent)]
    Handshake(HostError),
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

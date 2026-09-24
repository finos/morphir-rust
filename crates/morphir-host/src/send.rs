//! A `Send` bound that disappears on `wasm32`.

/// `Send` on native targets, and no bound on `wasm32`.
///
/// Browser values are not `Send`. Traits that a browser client implements use
/// this bound, so the same trait works in a native host and in a browser.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + ?Sized> MaybeSend for T {}

/// `Send` on native targets, and no bound on `wasm32`.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSend for T {}

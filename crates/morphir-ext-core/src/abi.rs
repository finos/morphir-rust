//! Core Wasm ABI definitions for Morphir extensions.

use crate::Envelope;

/// Represents a pointer to a memory region (Core Wasm ABI).
pub type Ptr = i32;

/// Represents the length of a memory region (Core Wasm ABI).
pub type Len = i32;

/// Host functions usually imported by the guest.
pub mod host {
    use super::{Len, Ptr};

    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        /// Log a message from the guest to the host.
        /// Log message is passed as a simple string.
        /// Level matches `crate::LogLevel`.
        pub fn log(level: i32, msg_ptr: Ptr, msg_len: Len);

        /// Get an environment variable.
        /// Result is written to `out_val_ptr` and `out_val_len`.
        /// Value is a JSON serialized `EnvValue`.
        pub fn get_env_var(name_ptr: Ptr, name_len: Len, out_val_ptr: Ptr, out_val_len: Ptr);

        /// Set an environment variable.
        /// Value is a JSON serialized `EnvValue`.
        pub fn set_env_var(name_ptr: Ptr, name_len: Len, val_ptr: Ptr, val_len: Len);
    }
}

/// Guest exports usually called by the host.
pub mod guest {
    use super::{Len, Ptr};

    /// The `start` function signature.
    /// Returns a program ID.
    pub type StartFn = unsafe extern "C" fn(
        hdr_ptr: Ptr,
        hdr_len: Len,
        ct_ptr: Ptr,
        ct_len: Len,
        msg_ptr: Ptr,
        msg_len: Len,
    ) -> u32;

    /// The `send` function signature.
    pub type SendFn = unsafe extern "C" fn(
        program_id: u32,
        hdr_ptr: Ptr,
        hdr_len: Len,
        ct_ptr: Ptr,
        ct_len: Len,
        msg_ptr: Ptr,
        msg_len: Len,
    );

    // Note: poll is more complex due to multiple return pointers,
    // usually handled via specific calling conventions or out-pointers.
}

/// Helper to decompose an Envelope into its raw pointer parts.
///
/// Returns (hdr_ptr, hdr_len, ct_ptr, ct_len, content_ptr, content_len).
/// Warning: This leaks memory. The types must be `std::mem::forget`-ed or
/// the pointers must be managed by the caller.
///
/// The header is serialized to JSON.
pub fn into_raw_parts(envelope: Envelope) -> (Ptr, Len, Ptr, Len, Ptr, Len) {
    let mut hdr = serde_json::to_vec(&envelope.header).unwrap_or_default();
    hdr.shrink_to_fit();

    // Safety: modifying the internal string/vec before getting pointers.

    // We need to consume content_type (String) and content (Vec).
    // String::into_bytes() returns Vec<u8>.
    let mut ct = envelope.content_type.into_bytes();
    ct.shrink_to_fit();

    let mut c = envelope.content;
    c.shrink_to_fit();

    let hdr_ptr = hdr.as_ptr() as Ptr;
    let hdr_len = hdr.len() as Len;
    let ct_ptr = ct.as_ptr() as Ptr;
    let ct_len = ct.len() as Len;
    let c_ptr = c.as_ptr() as Ptr;
    let c_len = c.len() as Len;

    // Prevent deallocation
    std::mem::forget(hdr);
    std::mem::forget(ct);
    std::mem::forget(c);

    (hdr_ptr, hdr_len, ct_ptr, ct_len, c_ptr, c_len)
}

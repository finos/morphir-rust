//! WASM exports for migrate extension.
//!
//! When compiled to WASM, this module provides the extension interface
//! that can be loaded by ExtismRuntime or other WASM hosts.

#[cfg(target_family = "wasm")]
use morphir_ext_core::{Envelope, abi};

#[cfg(target_family = "wasm")]
use crate::migrate::MigrateExtension;

#[cfg(target_family = "wasm")]
use crate::BuiltinExtension;

/// WASM export: backend-generate function for migrate extension.
///
/// This is called by the host runtime to perform IR migration.
#[cfg(target_family = "wasm")]
#[no_mangle]
pub extern "C" fn backend_generate(
    hdr_ptr: abi::Ptr,
    hdr_len: abi::Len,
    ct_ptr: abi::Ptr,
    ct_len: abi::Len,
    content_ptr: abi::Ptr,
    content_len: abi::Len,
) -> abi::Ptr {
    // Reconstruct envelope from raw parts
    let header_bytes =
        unsafe { std::slice::from_raw_parts(hdr_ptr as *const u8, hdr_len as usize) };
    let ct_bytes = unsafe { std::slice::from_raw_parts(ct_ptr as *const u8, ct_len as usize) };
    let content_bytes =
        unsafe { std::slice::from_raw_parts(content_ptr as *const u8, content_len as usize) };

    let header: morphir_ext_core::envelope::Header =
        serde_json::from_slice(header_bytes).expect("Failed to parse header");
    let content_type = String::from_utf8_lossy(ct_bytes).to_string();
    let content = content_bytes.to_vec();

    let input = Envelope {
        header,
        content_type,
        content,
    };

    // Execute native implementation
    let migrate = MigrateExtension::default();
    let output = migrate.execute_native(&input).expect("Migration failed");

    // Convert output to raw parts and return pointer
    let (hdr_ptr, _hdr_len, _ct_ptr, _ct_len, out_ptr, _out_len) = abi::into_raw_parts(output);

    // For now, just return the content pointer
    // TODO: Proper memory management for returning all parts
    out_ptr
}

/// WASM export: get-capabilities function.
///
/// Returns metadata about the migrate extension.
#[cfg(target_family = "wasm")]
#[no_mangle]
pub extern "C" fn get_capabilities() -> abi::Ptr {
    let migrate = MigrateExtension::default();
    let info = migrate.info();

    let capabilities = serde_json::json!({
        "id": info.id,
        "name": info.name,
        "type": info.extension_type.to_string(),
        "description": info.description,
    });

    let envelope = Envelope::json(&capabilities).expect("Failed to create envelope");
    let (_hdr_ptr, _hdr_len, _ct_ptr, _ct_len, out_ptr, _out_len) = abi::into_raw_parts(envelope);

    out_ptr
}

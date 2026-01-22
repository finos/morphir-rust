//! Host function imports for extensions
//!
//! Extensions can call these functions to interact with the host.

use crate::types::WorkspaceInfo;
use extism_pdk::*;

/// Log a message to the host
///
/// # Arguments
/// * `level` - Log level ("debug", "info", "warn", "error")
/// * `message` - Message to log
pub fn log(level: &str, message: &str) {
    // Use extism's built-in logging
    match level {
        "debug" => extism_pdk::debug!("{}", message),
        "info" => extism_pdk::info!("{}", message),
        "warn" => extism_pdk::warn!("{}", message),
        "error" => extism_pdk::error!("{}", message),
        _ => extism_pdk::info!("{}", message),
    }
}

/// Log a debug message
#[macro_export]
macro_rules! host_debug {
    ($($arg:tt)*) => {
        $crate::host::log("debug", &format!($($arg)*))
    };
}

/// Log an info message
#[macro_export]
macro_rules! host_info {
    ($($arg:tt)*) => {
        $crate::host::log("info", &format!($($arg)*))
    };
}

/// Log a warning message
#[macro_export]
macro_rules! host_warn {
    ($($arg:tt)*) => {
        $crate::host::log("warn", &format!($($arg)*))
    };
}

/// Log an error message
#[macro_export]
macro_rules! host_error {
    ($($arg:tt)*) => {
        $crate::host::log("error", &format!($($arg)*))
    };
}

// Host functions imported from the Morphir daemon
// These use JSON serialization for complex types

#[host_fn]
extern "ExtismHost" {
    /// Get workspace information from the host (returns JSON)
    fn morphir_get_workspace_info() -> Json<WorkspaceInfo>;
}

#[host_fn]
extern "ExtismHost" {
    /// Cache IR in the host
    fn morphir_cache_ir(key: String, ir: Json<serde_json::Value>);
}

#[host_fn]
extern "ExtismHost" {
    /// Get cached IR from the host
    fn morphir_get_cached_ir(key: String) -> Json<Option<serde_json::Value>>;
}

/// Get workspace information
///
/// Returns information about the current workspace including paths.
pub fn get_workspace_info() -> Option<WorkspaceInfo> {
    // This calls the host function; returns None if not available
    unsafe {
        morphir_get_workspace_info()
            .ok()
            .map(|json| json.into_inner())
    }
}

/// Cache IR in the host
///
/// The host may store this for later retrieval.
pub fn cache_ir(key: &str, ir: &serde_json::Value) {
    unsafe {
        let _ = morphir_cache_ir(key.to_string(), Json(ir.clone()));
    }
}

/// Get cached IR from the host
pub fn get_cached_ir(key: &str) -> Option<serde_json::Value> {
    unsafe {
        morphir_get_cached_ir(key.to_string())
            .ok()
            .and_then(|json| json.into_inner())
    }
}

/// Get the value of a configuration variable from the host
pub fn get_config(key: &str) -> Option<String> {
    extism_pdk::config::get(key).ok().flatten()
}

/// Get a variable from the host
pub fn get_var(key: &str) -> Option<String> {
    extism_pdk::var::get(key)
        .ok()
        .flatten()
        .map(|b| String::from_utf8(b).unwrap_or_default())
}

/// Set a variable in the host
pub fn set_var(key: &str, value: &str) {
    let _ = extism_pdk::var::set(key, value.as_bytes());
}

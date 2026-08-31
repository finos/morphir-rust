//! Extension container using Extism
//!
//! This module provides the runtime container for loaded extensions.

use crate::error::{DaemonError, Result};
use crate::extensions::host_functions::MorphirHostFunctions;
use crate::extensions::protocol::{
    ExtensionRequest, ExtensionResponse, ExtensionResponseExt, MAX_MEP_PAYLOAD_BYTES,
};
use extism::{Manifest, Plugin, PluginBuilder, Wasm};
pub use morphir_extension_sdk::{ExtensionInfo, ExtensionType};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, info};

const MAX_WASM_MEMORY_BYTES: u64 = 256 * 1024 * 1024;
const WASM_PAGE_BYTES: u64 = 64 * 1024;
const DEFAULT_WASM_EXECUTION_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_WASM_FUEL_LIMIT: u64 = 100_000_000;

fn wasm_pages_for_bytes(bytes: u64) -> Result<u32> {
    if !bytes.is_multiple_of(WASM_PAGE_BYTES) {
        return Err(DaemonError::Extension(format!(
            "WASM memory limit {bytes} bytes is not a whole number of WebAssembly pages"
        )));
    }
    let pages = bytes.checked_div(WASM_PAGE_BYTES).ok_or_else(|| {
        DaemonError::Extension("WebAssembly page size must be non-zero".to_string())
    })?;
    u32::try_from(pages).map_err(|_| {
        DaemonError::Extension(format!(
            "WASM memory limit {bytes} bytes exceeds the Extism page limit"
        ))
    })
}

fn ensure_payload_within_limit(direction: &str, payload_bytes: usize) -> Result<()> {
    if payload_bytes > MAX_MEP_PAYLOAD_BYTES as usize {
        return Err(DaemonError::Extension(format!(
            "Extension {direction} exceeds the {MAX_MEP_PAYLOAD_BYTES} byte limit"
        )));
    }
    Ok(())
}

/// Container for a loaded extension plugin
pub struct ExtensionContainer {
    /// Extension identifier
    id: String,
    /// The Extism plugin instance
    plugin: Arc<Mutex<Plugin>>,
    /// Extension metadata
    info: ExtensionInfo,
    /// Request ID counter
    request_id: std::sync::atomic::AtomicU64,
}

impl ExtensionContainer {
    /// Create a new extension container from a WASM file
    pub fn new(id: &str, wasm_path: &Path, host_funcs: MorphirHostFunctions) -> Result<Self> {
        info!("Loading extension '{}' from {:?}", id, wasm_path);

        // Read the WASM file
        let wasm_bytes = std::fs::read(wasm_path)?;

        Self::from_bytes(id, &wasm_bytes, host_funcs)
    }

    /// Create a new extension container from WASM bytes
    pub fn from_bytes(
        id: &str,
        wasm_bytes: &[u8],
        host_funcs: MorphirHostFunctions,
    ) -> Result<Self> {
        Self::from_bytes_with_timeout(id, wasm_bytes, host_funcs, DEFAULT_WASM_EXECUTION_TIMEOUT)
    }

    /// Create a container from WASM bytes owned by the blocking compilation worker.
    pub async fn from_bytes_async<B>(
        id: String,
        wasm_bytes: B,
        host_funcs: MorphirHostFunctions,
    ) -> Result<Self>
    where
        B: AsRef<[u8]> + Send + 'static,
    {
        Self::from_bytes_with_limits_async(
            id,
            wasm_bytes,
            host_funcs,
            DEFAULT_WASM_EXECUTION_TIMEOUT,
            DEFAULT_WASM_FUEL_LIMIT,
        )
        .await
    }

    fn from_bytes_with_timeout(
        id: &str,
        wasm_bytes: &[u8],
        host_funcs: MorphirHostFunctions,
        execution_timeout: Duration,
    ) -> Result<Self> {
        Self::from_bytes_with_limits(
            id,
            wasm_bytes,
            host_funcs,
            execution_timeout,
            DEFAULT_WASM_FUEL_LIMIT,
        )
    }

    async fn from_bytes_with_limits_async<B>(
        id: String,
        wasm_bytes: B,
        host_funcs: MorphirHostFunctions,
        execution_timeout: Duration,
        fuel_limit: u64,
    ) -> Result<Self>
    where
        B: AsRef<[u8]> + Send + 'static,
    {
        tokio::task::spawn_blocking(move || {
            Self::from_bytes_with_limits(
                &id,
                wasm_bytes.as_ref(),
                host_funcs,
                execution_timeout,
                fuel_limit,
            )
        })
        .await
        .map_err(|error| {
            DaemonError::Extension(format!("Extension discovery worker failed: {error}"))
        })?
    }

    fn from_bytes_with_limits(
        id: &str,
        wasm_bytes: &[u8],
        host_funcs: MorphirHostFunctions,
        execution_timeout: Duration,
        fuel_limit: u64,
    ) -> Result<Self> {
        // Create manifest with memory limits
        let manifest = Manifest::new([Wasm::data(wasm_bytes)])
            .with_memory_max(wasm_pages_for_bytes(MAX_WASM_MEMORY_BYTES)?)
            .with_timeout(execution_timeout);

        // Create plugin with host functions
        let mut plugin = PluginBuilder::new(manifest)
            .with_functions(host_funcs.into_functions())
            .with_wasi(true)
            .with_fuel_limit(fuel_limit)
            .build()
            .map_err(|e| DaemonError::Extension(format!("Failed to create plugin: {}", e)))?;

        // Query extension info
        let info: ExtensionInfo = {
            let output = plugin
                .call::<&[u8], &[u8]>("morphir_extension_info", &[])
                .map_err(|e| {
                    DaemonError::Extension(format!("Failed to get extension info: {}", e))
                })?;
            ensure_payload_within_limit("response", output.len())?;
            serde_json::from_slice(output)?
        };

        debug!("Loaded extension: {} v{}", info.name, info.version);

        Ok(Self {
            id: id.to_string(),
            plugin: Arc::new(Mutex::new(plugin)),
            info,
            request_id: std::sync::atomic::AtomicU64::new(1),
        })
    }

    /// Get extension info
    pub fn info(&self) -> &ExtensionInfo {
        &self.info
    }

    /// Get extension ID
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Check if extension supports a capability
    pub fn supports(&self, ext_type: ExtensionType) -> bool {
        self.info.types.contains(&ext_type)
    }

    /// Call an extension method with JSON-RPC
    pub async fn call<I: Serialize, O: DeserializeOwned>(
        &self,
        method: &str,
        params: I,
    ) -> Result<O> {
        let id = self
            .request_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let request = ExtensionRequest::new(method, params, id)?;
        let request_bytes = serde_json::to_vec(&request)?;

        debug!("Calling extension method: {} (id={})", method, id);

        let output = self.call_raw("handle", &request_bytes).await?;

        let response: ExtensionResponse = serde_json::from_slice(&output)?;
        response.into_result(id)
    }

    /// Call a raw function on the plugin (no JSON-RPC wrapping)
    pub async fn call_raw(&self, func_name: &str, input: &[u8]) -> Result<Vec<u8>> {
        ensure_payload_within_limit("request", input.len())?;
        let mut plugin = Arc::clone(&self.plugin).lock_owned().await;
        let func_name = func_name.to_owned();
        let input = input.to_vec();
        tokio::task::spawn_blocking(move || {
            let output = plugin
                .call::<&[u8], &[u8]>(&func_name, &input)
                .map_err(|e| DaemonError::Extension(format!("Plugin call failed: {}", e)))?;
            ensure_payload_within_limit("response", output.len())?;
            Ok(output.to_vec())
        })
        .await
        .map_err(|error| {
            DaemonError::Extension(format!("Extension plugin worker failed: {error}"))
        })?
    }
}

/// Builder for ExtensionContainer with configuration options
pub struct ExtensionContainerBuilder {
    id: String,
    wasm_path: Option<std::path::PathBuf>,
    wasm_bytes: Option<Vec<u8>>,
    host_funcs: Option<MorphirHostFunctions>,
    config: HashMap<String, String>,
}

impl ExtensionContainerBuilder {
    /// Create a new builder
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            wasm_path: None,
            wasm_bytes: None,
            host_funcs: None,
            config: HashMap::new(),
        }
    }

    /// Set WASM file path
    pub fn with_path(mut self, path: impl AsRef<Path>) -> Self {
        self.wasm_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Set WASM bytes
    pub fn with_bytes(mut self, bytes: Vec<u8>) -> Self {
        self.wasm_bytes = Some(bytes);
        self
    }

    /// Set host functions
    pub fn with_host_functions(mut self, funcs: MorphirHostFunctions) -> Self {
        self.host_funcs = Some(funcs);
        self
    }

    /// Add configuration value
    pub fn with_config(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.config.insert(key.into(), value.into());
        self
    }

    /// Build the extension container
    pub fn build(self) -> Result<ExtensionContainer> {
        let host_funcs = self.host_funcs.unwrap_or_default();

        if let Some(path) = self.wasm_path {
            ExtensionContainer::new(&self.id, &path, host_funcs)
        } else if let Some(bytes) = self.wasm_bytes {
            ExtensionContainer::from_bytes(&self.id, &bytes, host_funcs)
        } else {
            Err(DaemonError::Extension(
                "No WASM path or bytes provided".into(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn writes(output: &str) -> String {
        output
            .bytes()
            .enumerate()
            .map(|(index, byte)| {
                format!(
                    "(call $store_u8 (i64.add (local.get $output) (i64.const {index})) (i32.const {byte}))"
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn guest_with_handle(handle_body: &str) -> Vec<u8> {
        let info = serde_json::json!({
            "id": "runtime-limit-fixture",
            "name": "Runtime Limit Fixture",
            "version": "1.0.0",
            "types": ["backend"]
        })
        .to_string();
        let info_writes = writes(&info);
        wat::parse_str(format!(
            r#"(module
                (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
                (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
                (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
                (func (export "morphir_extension_info") (result i32)
                    (local $output i64)
                    (local.set $output (call $alloc (i64.const {info_length})))
                    {info_writes}
                    (call $output_set (local.get $output) (i64.const {info_length}))
                    (i32.const 0))
                (func (export "handle") (result i32)
                    {handle_body}))"#,
            info_length = info.len(),
        ))
        .unwrap()
    }

    fn guest_with_small_handle_response() -> Vec<u8> {
        let response = "{}";
        let response_writes = writes(response);
        guest_with_handle(&format!(
            r#"(local $output i64)
                (local.set $output (call $alloc (i64.const {response_length})))
                {response_writes}
                (call $output_set (local.get $output) (i64.const {response_length}))
                (i32.const 0)"#,
            response_length = response.len(),
        ))
    }

    fn guest_with_handle_response_size(response_bytes: usize) -> Vec<u8> {
        guest_with_handle(&format!(
            r#"(local $output i64)
                (local.set $output (call $alloc (i64.const {response_bytes})))
                (call $output_set (local.get $output) (i64.const {response_bytes}))
                (i32.const 0)"#,
        ))
    }

    fn guest_with_extension_info_size(info_bytes: usize) -> Vec<u8> {
        wat::parse_str(format!(
            r#"(module
                (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
                (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
                (func (export "morphir_extension_info") (result i32)
                    (local $output i64)
                    (local.set $output (call $alloc (i64.const {info_bytes})))
                    (call $output_set (local.get $output) (i64.const {info_bytes}))
                    (i32.const 0))
                (func (export "handle") (result i32)
                    (i32.const 0)))"#,
        ))
        .unwrap()
    }

    fn guest_with_non_terminating_handle() -> Vec<u8> {
        guest_with_handle(
            r#"(loop $forever
                    (br $forever))
                (i32.const 0)"#,
        )
    }

    fn guest_with_non_terminating_discovery() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (func (export "morphir_extension_info") (result i32)
                    (loop $forever
                        (br $forever))
                    (i32.const 0))
                (func (export "handle") (result i32)
                    (i32.const 0)))"#,
        )
        .unwrap()
    }

    fn guest_attempting_to_exceed_memory_limit() -> Vec<u8> {
        let info = serde_json::json!({
            "id": "runtime-limit-fixture",
            "name": "Runtime Limit Fixture",
            "version": "1.0.0",
            "types": ["backend"]
        })
        .to_string();
        let info_writes = writes(&info);
        wat::parse_str(format!(
            r#"(module
                (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
                (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
                (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
                (memory 1)
                (func (export "morphir_extension_info") (result i32)
                    (local $output i64)
                    (local.set $output (call $alloc (i64.const {info_length})))
                    {info_writes}
                    (call $output_set (local.get $output) (i64.const {info_length}))
                    (i32.const 0))
                (func (export "handle") (result i32)
                    (if (i32.ne (memory.grow (i32.const 4097)) (i32.const -1))
                        (then unreachable))
                    (i32.const 0)))"#,
            info_length = info.len(),
        ))
        .unwrap()
    }

    #[test]
    fn test_extension_type_serde() {
        let json = serde_json::to_string(&ExtensionType::Frontend).unwrap();
        assert_eq!(json, "\"frontend\"");

        let parsed: ExtensionType = serde_json::from_str("\"backend\"").unwrap();
        assert_eq!(parsed, ExtensionType::Backend);
    }

    #[test]
    fn configures_the_256_mib_memory_limit_in_wasm_pages() {
        assert_eq!(wasm_pages_for_bytes(MAX_WASM_MEMORY_BYTES).unwrap(), 4096);
    }

    #[tokio::test]
    async fn enforces_the_256_mib_memory_limit_on_guest_growth() {
        let container = ExtensionContainer::from_bytes(
            "runtime-limit-fixture",
            &guest_attempting_to_exceed_memory_limit(),
            MorphirHostFunctions::default(),
        )
        .unwrap();

        let error = container
            .call_raw("handle", &[])
            .await
            .expect_err("Extism should reject growth beyond 4096 WebAssembly pages");

        assert!(error.to_string().contains("oom"));
    }

    #[tokio::test]
    async fn rejects_handle_requests_above_the_mep_payload_limit() {
        let container = ExtensionContainer::from_bytes(
            "runtime-limit-fixture",
            &guest_with_small_handle_response(),
            MorphirHostFunctions::default(),
        )
        .unwrap();
        let request = vec![0; MAX_MEP_PAYLOAD_BYTES as usize + 1];

        let error = container
            .call_raw("handle", &request)
            .await
            .expect_err("an oversized handle request should be rejected");

        assert!(error.to_string().contains("request exceeds the"));
        assert!(error.to_string().contains("byte limit"));
    }

    #[tokio::test]
    async fn rejects_handle_responses_above_the_mep_payload_limit() {
        let container = ExtensionContainer::from_bytes(
            "runtime-limit-fixture",
            &guest_with_handle_response_size(MAX_MEP_PAYLOAD_BYTES as usize + 1),
            MorphirHostFunctions::default(),
        )
        .unwrap();

        let error = container
            .call_raw("handle", &[])
            .await
            .expect_err("an oversized handle response should be rejected");

        assert!(error.to_string().contains("response exceeds the"));
        assert!(error.to_string().contains("byte limit"));
    }

    #[test]
    fn rejects_discovery_responses_above_the_mep_payload_limit() {
        let error = ExtensionContainer::from_bytes(
            "runtime-limit-fixture",
            &guest_with_extension_info_size(MAX_MEP_PAYLOAD_BYTES as usize + 1),
            MorphirHostFunctions::default(),
        )
        .err()
        .expect("an oversized discovery response should be rejected");

        assert!(error.to_string().contains("response exceeds the"));
        assert!(error.to_string().contains("byte limit"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn interrupts_non_terminating_handles_without_blocking_tokio() {
        let container = ExtensionContainer::from_bytes_with_timeout(
            "runtime-limit-fixture",
            &guest_with_non_terminating_handle(),
            MorphirHostFunctions::default(),
            Duration::from_millis(50),
        )
        .unwrap();
        let heartbeat = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&heartbeat);
        let ticker = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(5)).await;
                observed.fetch_add(1, Ordering::SeqCst);
            }
        });
        tokio::task::yield_now().await;
        heartbeat.store(0, Ordering::SeqCst);

        let error = tokio::time::timeout(Duration::from_secs(1), container.call_raw("handle", &[]))
            .await
            .expect("the host-side deadline should remain schedulable")
            .expect_err("the non-terminating guest should be interrupted");
        ticker.abort();

        let error = error.to_string();
        assert!(error.contains("timeout") || error.contains("fuel"));
        assert!(
            heartbeat.load(Ordering::SeqCst) > 0,
            "Extism execution blocked the Tokio runtime thread"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn discovery_does_not_block_tokio_runtime_threads() {
        let mut discovery = Box::pin(ExtensionContainer::from_bytes_with_limits_async(
            "runtime-limit-fixture".to_string(),
            guest_with_non_terminating_discovery(),
            MorphirHostFunctions::default(),
            Duration::from_millis(100),
            u64::MAX,
        ));
        let mut scheduler_progress = Box::pin(tokio::task::yield_now());

        tokio::select! {
            biased;
            result = &mut discovery => {
                match result {
                    Ok(_) => panic!("discovery completed before the runtime could reschedule"),
                    Err(error) => panic!(
                        "discovery failed before the runtime could reschedule: {error}"
                    ),
                }
            }
            () = &mut scheduler_progress => {}
        }

        let result = tokio::time::timeout(Duration::from_secs(1), discovery)
            .await
            .expect("the host-side deadline should remain schedulable");
        let error = result
            .err()
            .expect("the non-terminating discovery guest should be interrupted");

        let error = error.to_string();
        assert!(error.contains("timeout") || error.contains("fuel"));
    }

    #[tokio::test]
    async fn exhausts_the_fuel_budget_for_non_terminating_guests() {
        let container = ExtensionContainer::from_bytes_with_timeout(
            "runtime-limit-fixture",
            &guest_with_non_terminating_handle(),
            MorphirHostFunctions::default(),
            Duration::from_secs(2),
        )
        .unwrap();

        let error = container
            .call_raw("handle", &[])
            .await
            .expect_err("the non-terminating guest should exhaust its fuel");

        assert!(error.to_string().contains("fuel"));
    }
}

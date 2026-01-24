---
layout: default
title: Extension Host Interface
nav_order: 4
parent: Morphir Extensions
---

# Extension Host Interface

**Status:** Draft  
**Version:** 0.1.0

## Overview

The `ExtensionHost` trait defines the common interface that all protocol-specific hosts must implement. This abstraction allows the Extension Manager to work with any host type uniformly.

## Trait Definition

```rust
use async_trait::async_trait;
use serde_json::Value;

#[async_trait]
pub trait ExtensionHost: Send + 'static {
    /// Returns the protocol identifier (e.g., "jsonrpc2", "grpc", "stdio")
    fn protocol(&self) -> &str;

    /// Initialize the host (called once when host is created)
    async fn initialize(&mut self) -> Result<(), ExtensionError>;

    /// Load an extension from the given configuration
    async fn load_extension(
        &mut self,
        config: ExtensionConfig,
    ) -> Result<ExtensionId, ExtensionError>;

    /// Call a method on a loaded extension
    async fn call(
        &mut self,
        extension_id: ExtensionId,
        method: &str,
        params: Value,
    ) -> Result<Value, ExtensionError>;

    /// Unload an extension and clean up resources
    async fn unload_extension(
        &mut self,
        extension_id: ExtensionId,
    ) -> Result<(), ExtensionError>;

    /// Query capabilities of a loaded extension
    async fn capabilities(
        &self,
        extension_id: ExtensionId,
    ) -> Result<Vec<Capability>, ExtensionError>;

    /// Check if extension is healthy (optional, default returns true)
    async fn health_check(
        &self,
        extension_id: ExtensionId,
    ) -> Result<HealthStatus, ExtensionError> {
        Ok(HealthStatus::Healthy)
    }
}
```

## Common Types

### ExtensionId

Unique identifier for a loaded extension:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExtensionId(pub u64);
```

### ExtensionConfig

Configuration for loading an extension:

```rust
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExtensionConfig {
    /// Human-readable name
    pub name: String,

    /// Source location/configuration
    pub source: ExtensionSource,

    /// Permissions granted to extension
    pub permissions: Permissions,

    /// Extension-specific configuration
    pub config: Value,

    /// Restart strategy on failure
    #[serde(default)]
    pub restart: RestartStrategy,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ExtensionSource {
    Http { url: String },
    Grpc { endpoint: String },
    Process {
        command: String,
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    Wasm { path: PathBuf },
    WasmComponent { path: PathBuf },
}
```

### Capability

Description of what an extension can do:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capability {
    /// Method name
    pub name: String,

    /// Human-readable description
    pub description: String,

    /// JSON schema for parameters (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params_schema: Option<Value>,

    /// JSON schema for return value (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_schema: Option<Value>,
}
```

### Permissions

Permissions granted to extension:

```rust
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Permissions {
    /// Allow network access
    #[serde(default)]
    pub network: bool,

    /// Allowed filesystem paths
    #[serde(default)]
    pub filesystem: Vec<PathBuf>,

    /// Maximum memory (WASM only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_memory: Option<String>, // e.g., "100MB"

    /// Maximum execution time per call
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_execution_time: Option<Duration>,
}
```

### HealthStatus

Health status of an extension:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Degraded { reason: String },
    Unhealthy { reason: String },
}
```

### ExtensionError

Common error type for all host operations:

```rust
#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    #[error("Extension not found: {0}")]
    NotFound(ExtensionId),

    #[error("Invalid extension source")]
    InvalidSource,

    #[error("Initialization failed: {0}")]
    InitializationFailed(String),

    #[error("Method not found: {0}")]
    MethodNotFound(String),

    #[error("Extension error: {0}")]
    ExtensionError(String),

    #[error("Protocol error: {0}")]
    ProtocolError(String),

    #[error("Timeout")]
    Timeout,

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}
```

## Lifecycle Methods

### initialize()

Called once when the host actor is created. Use for:

- Setting up global state
- Initializing protocol clients
- Loading configuration

**Example:**

```rust
async fn initialize(&mut self) -> Result<(), ExtensionError> {
    tracing::info!("Initializing {} host", self.protocol());
    // Setup protocol-specific state
    Ok(())
}
```

### load_extension()

Loads a new extension. Must:

1. Validate configuration
2. Spawn/connect to extension
3. Send initialization message
4. Query capabilities
5. Return unique ExtensionId

**Sequence:**

```
Host receives config
  ↓
Validate source matches protocol
  ↓
Spawn/connect to extension
  ↓
Send "initialize" with config.config
  ↓
Wait for success response
  ↓
Send "capabilities" query
  ↓
Store extension metadata
  ↓
Return ExtensionId
```

### call()

Invokes a method on an extension. Must:

1. Validate extension exists
2. Serialize parameters
3. Send request with timeout
4. Deserialize response
5. Return result or error

**Error Handling:**

- If extension not found → `ExtensionError::NotFound`
- If method unknown → `ExtensionError::MethodNotFound`
- If timeout → `ExtensionError::Timeout`
- If extension returns error → `ExtensionError::ExtensionError`

### unload_extension()

Cleanly shuts down an extension. Must:

1. Send shutdown notification (if protocol supports)
2. Wait briefly for cleanup
3. Force terminate if needed
4. Remove from tracking

**Best Effort:**
Should not fail even if extension already dead.

### capabilities()

Returns cached capabilities. Should be fast (no RPC).

## Implementation Guidelines

### Thread Safety

All methods receive `&mut self`, so:

- **No internal locking needed** (actor guarantees sequential access)
- **Can mutate state freely**
- **Blocking operations must use spawn_blocking**

### Timeouts

All operations with external communication should use timeouts:

```rust
tokio::time::timeout(
    Duration::from_secs(30),
    some_async_operation()
).await??
```

### Error Propagation

Use `?` operator and convert to `ExtensionError`:

```rust
let response = client.call(request).await
    .map_err(|e| ExtensionError::ProtocolError(e.to_string()))?;
```

### Logging

Use structured logging with `tracing`:

```rust
#[instrument(skip(self), fields(extension_id = %extension_id))]
async fn call(&mut self, extension_id: ExtensionId, ...) {
    debug!("Calling method", method = %method);
    // ...
}
```

### Metrics

Emit metrics for all operations:

```rust
let start = Instant::now();
let result = self.call_internal(id, method, params).await;
let duration = start.elapsed();

metrics::histogram!("morphir.extension.call.duration_ms")
    .record(duration.as_millis() as f64);

metrics::counter!("morphir.extension.call.count")
    .increment(1);
```

## Testing

Host implementations should include:

1. **Unit tests**: Test individual methods
2. **Integration tests**: Test with real/mock extensions
3. **Failure tests**: Test error handling
4. **Concurrency tests**: Multiple calls in parallel
5. **Timeout tests**: Verify timeout behavior

**Example:**

```rust
#[tokio::test]
async fn test_load_and_call() {
    let mut host = StdioExtensionHost::new();
    host.initialize().await.unwrap();

    let config = ExtensionConfig {
        name: "test".into(),
        source: ExtensionSource::Process {
            command: "python3".into(),
            args: vec!["./test_extension.py".into()],
            env: HashMap::new(),
        },
        permissions: Permissions::default(),
        config: json!({}),
        restart: RestartStrategy::Never,
    };

    let id = host.load_extension(config).await.unwrap();

    let result = host.call(
        id,
        "echo",
        json!({"message": "hello"}),
    ).await.unwrap();

    assert_eq!(result, json!({"message": "hello"}));
}
```

## Related

### Morphir Rust Design Documents

- **[Morphir Extensions](../README.md)** - Extension system overview
- **[WASM Components](../wasm-component.md)** - Component model integration
- **[Tasks](../tasks.md)** - Task system definition

### Main Morphir Documentation

- [Morphir Documentation](https://morphir.finos.org) - Main Morphir documentation site
- [Morphir LLMs.txt](https://morphir.finos.org/llms.txt) - Machine-readable documentation index
- [Morphir IR v4 Design](https://morphir.finos.org/docs/design/draft/ir/) - IR v4 design documents
- [Morphir IR Specification](https://morphir.finos.org/docs/morphir-ir-specification/) - Complete IR specification

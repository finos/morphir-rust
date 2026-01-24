---
layout: default
title: Extension Manager
nav_order: 10
parent: Morphir Extensions
---

# Extension Manager

**Status:** Draft  
**Version:** 0.1.0

## Overview

The Extension Manager is the central coordinator for all extensions in Morphir. It provides a unified API for loading, calling, and managing extensions regardless of their underlying protocol.

## Architecture

```
┌─────────────────────────────────────────┐
│        Extension Manager                │
│        (Kameo Actor)                    │
│                                         │
│  ┌────────────────────────────────┐   │
│  │  hosts: HashMap<String, Host>   │   │
│  │    "jsonrpc2" → JsonRpcHost     │   │
│  │    "grpc"     → GrpcHost        │   │
│  │    "stdio"    → StdioHost       │   │
│  │    "extism"   → ExtismHost      │   │
│  │    "component"→ ComponentHost   │   │
│  └────────────────────────────────┘   │
│                                         │
│  ┌────────────────────────────────┐   │
│  │  extensions: HashMap            │   │
│  │    ExtensionId → Metadata       │   │
│  └────────────────────────────────┘   │
│                                         │
│  ┌────────────────────────────────┐   │
│  │  registry: HashMap              │   │
│  │    String → ExtensionId         │   │
│  │    (name lookup)                │   │
│  └────────────────────────────────┘   │
└─────────────────────────────────────────┘
```

## State Management

```rust
#[derive(Actor)]
pub struct ExtensionManager {
    /// Protocol hosts, keyed by protocol name
    hosts: HashMap<String, Box<dyn ExtensionHostActor>>,

    /// Metadata for all loaded extensions
    extensions: HashMap<ExtensionId, ExtensionMetadata>,

    /// Name-to-ID registry for fast lookup
    registry: HashMap<String, ExtensionId>,

    /// Event bus for publishing events
    event_bus: ActorRef<PubSub>,

    /// Next extension ID
    next_id: u64,
}

struct ExtensionMetadata {
    id: ExtensionId,
    name: String,
    protocol: String,
    capabilities: Vec<Capability>,
    config: ExtensionConfig,
    loaded_at: Instant,
    call_count: u64,
    error_count: u64,
}
```

## Messages

### LoadExtension

Loads a new extension from configuration.

```rust
#[message]
pub async fn load_extension(
    &mut self,
    config: ExtensionConfig,
) -> Result<ExtensionId, ExtensionError> {
    // 1. Determine protocol from source
    let protocol = match &config.source {
        ExtensionSource::Http { .. } => "jsonrpc2",
        ExtensionSource::Grpc { .. } => "grpc",
        ExtensionSource::Process { .. } => "stdio",
        ExtensionSource::Wasm { .. } => "extism",
        ExtensionSource::WasmComponent { .. } => "component",
    };

    // 2. Get host
    let host = self.hosts
        .get_mut(protocol)
        .ok_or(ExtensionError::HostNotFound)?;

    // 3. Load in host
    let id = host.load(config.clone()).await?;

    // 4. Store metadata
    self.extensions.insert(id, ExtensionMetadata {
        id,
        name: config.name.clone(),
        protocol: protocol.to_string(),
        capabilities: vec![], // Fetched from host
        config,
        loaded_at: Instant::now(),
        call_count: 0,
        error_count: 0,
    });

    // 5. Register name
    self.registry.insert(config.name, id);

    // 6. Publish event
    self.event_bus.tell(Publish::new("extension.loaded", id)).await?;

    Ok(id)
}
```

### CallExtension

Calls a method on an extension by name.

```rust
#[message]
pub async fn call_extension(
    &mut self,
    name: String,
    method: String,
    params: Value,
) -> Result<Value, ExtensionError> {
    // 1. Resolve name → ID
    let id = self.registry
        .get(&name)
        .copied()
        .ok_or_else(|| ExtensionError::NotFound(name))?;

    // 2. Get metadata
    let metadata = self.extensions
        .get_mut(&id)
        .ok_or(ExtensionError::NotFound(id))?;

    // 3. Get host
    let host = self.hosts
        .get_mut(&metadata.protocol)
        .ok_or(ExtensionError::HostNotFound)?;

    // 4. Call extension
    metadata.call_count += 1;

    let start = Instant::now();
    let result = host.call(id, method, params).await;
    let duration = start.elapsed();

    // 5. Update metrics
    if result.is_err() {
        metadata.error_count += 1;
    }

    metrics::histogram!("morphir.extension.call.duration_ms")
        .record(duration.as_millis() as f64);

    result
}
```

### ListExtensions

Lists all loaded extensions.

```rust
#[message]
pub fn list_extensions(&self) -> Vec<ExtensionInfo> {
    self.extensions
        .values()
        .map(|meta| ExtensionInfo {
            id: meta.id,
            name: meta.name.clone(),
            protocol: meta.protocol.clone(),
            capabilities: meta.capabilities.clone(),
            call_count: meta.call_count,
            error_count: meta.error_count,
            uptime: meta.loaded_at.elapsed(),
        })
        .collect()
}
```

### UnloadExtension

Unloads an extension.

```rust
#[message]
pub async fn unload_extension(
    &mut self,
    name: String,
) -> Result<(), ExtensionError> {
    let id = self.registry
        .remove(&name)
        .ok_or_else(|| ExtensionError::NotFound(name))?;

    let metadata = self.extensions
        .remove(&id)
        .ok_or(ExtensionError::NotFound(id))?;

    let host = self.hosts
        .get_mut(&metadata.protocol)
        .ok_or(ExtensionError::HostNotFound)?;

    host.unload(id).await?;

    self.event_bus.tell(Publish::new("extension.unloaded", id)).await?;

    Ok(())
}
```

## Usage Example

```rust
#[tokio::main]
async fn main() -> Result<()> {
    // Create manager
    let manager = ExtensionManager::new().spawn();

    // Register hosts
    manager.tell(RegisterHost {
        protocol: "stdio".to_string(),
        host: StdioExtensionHost::new().spawn(),
    }).await?;

    manager.tell(RegisterHost {
        protocol: "jsonrpc2".to_string(),
        host: JsonRpcExtensionHost::new().spawn(),
    }).await?;

    // Load extension
    let config = ExtensionConfig {
        name: "my-extension".to_string(),
        source: ExtensionSource::Process {
            command: "python3".to_string(),
            args: vec!["./extension.py".to_string()],
            env: HashMap::new(),
        },
        permissions: Permissions::default(),
        config: json!({}),
        restart: RestartStrategy::Never,
    };

    let id = manager.ask(LoadExtension(config)).await?;

    // Call extension
    let result = manager.ask(CallExtension {
        name: "my-extension".to_string(),
        method: "transform".to_string(),
        params: json!({"ir": {...}}),
    }).await?;

    // List extensions
    let extensions = manager.ask(ListExtensions).await?;
    for ext in extensions {
        println!("{}: {} calls", ext.name, ext.call_count);
    }

    Ok(())
}
```

## Configuration Loading

The manager can load extensions from a configuration file:

```rust
impl ExtensionManager {
    pub async fn load_from_config(&mut self, path: &Path) -> Result<()> {
        let config: ExtensionConfig = toml::from_str(&fs::read_to_string(path)?)?;

        for ext_config in config.extensions {
            if ext_config.enabled {
                self.load_extension(ext_config).await?;
            }
        }

        Ok(())
    }
}
```

## Supervision

The manager implements supervision for host failures:

```rust
impl Actor for ExtensionManager {
    async fn on_link_died(
        &mut self,
        actor_ref: WeakActorRef<Self>,
        id: ActorID,
        reason: ActorStopReason,
    ) {
        // A host died - attempt to restart
        warn!("Host died: {:?}", reason);

        // Find which host died and restart extensions
        // Implementation depends on restart strategy
    }
}
```

## Events

The manager publishes these events via the event bus:

- `extension.loaded` - Extension successfully loaded
- `extension.failed` - Extension failed to load
- `extension.unloaded` - Extension unloaded
- `extension.call.started` - Method call started
- `extension.call.completed` - Method call completed
- `extension.call.failed` - Method call failed
- `host.registered` - New host registered
- `host.failed` - Host actor failed

## Metrics

The manager emits these metrics:

- `morphir.extension.count` - Number of loaded extensions
- `morphir.extension.load.duration_ms` - Time to load extension
- `morphir.extension.call.duration_ms` - Time to call method
- `morphir.extension.call.count` - Number of calls (by extension, method, status)
- `morphir.extension.error.count` - Number of errors (by extension)

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

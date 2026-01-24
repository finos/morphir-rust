---
layout: default
title: Morphir Extension System - Architecture
nav_order: 3
parent: Morphir Extensions
---

# Morphir Extension System - Architecture

**Status:** Draft  
**Version:** 0.1.0

## System Overview

```
┌─────────────────────────────────────────────────────────────┐
│                     Morphir Core                            │
│  ┌─────────────┐  ┌──────────┐  ┌────────────┐            │
│  │  Compiler   │  │    CLI   │  │  Codegen   │            │
│  └──────┬──────┘  └────┬─────┘  └─────┬──────┘            │
│         │              │               │                    │
│         └──────────────┴───────────────┘                    │
│                        │                                    │
│                        ▼                                    │
│         ┌──────────────────────────┐                       │
│         │   Extension Manager      │                       │
│         │   (Kameo Actor)          │                       │
│         └──────────┬───────────────┘                       │
│                    │                                        │
└────────────────────┼────────────────────────────────────────┘
                     │
        ┌────────────┼────────────────┐
        │            │                │
        ▼            ▼                ▼
┌──────────────┐ ┌──────────────┐ ┌──────────────┐
│  JSON-RPC    │ │   gRPC       │ │   Stdio      │
│  Host Actor  │ │   Host Actor │ │   Host Actor │
└──────┬───────┘ └──────┬───────┘ └──────┬───────┘
       │                │                │
       ▼                ▼                ▼
┌────────────┐   ┌────────────┐   ┌────────────┐
│ Extension  │   │ Extension  │   │ Extension  │
│ (HTTP)     │   │ (gRPC)     │   │ (Process)  │
└────────────┘   └────────────┘   └────────────┘

┌──────────────┐ ┌──────────────┐
│  Extism WASM │ │   Component  │
│  Host Actor  │ │   Host Actor │
└──────┬───────┘ └──────┬───────┘
       │                │
       ▼                ▼
┌────────────┐   ┌────────────┐
│ Extension  │   │ Extension  │
│ (WASM)     │   │ (Component)│
└────────────┘   └────────────┘
```

## Component Responsibilities

### Extension Manager

**Type**: Kameo Actor  
**Lifecycle**: Singleton, lives for application duration

**Responsibilities:**

- Register and manage extension hosts
- Route extension calls to appropriate host
- Maintain extension registry (name → ID → host)
- Handle extension discovery and loading
- Provide unified query interface (list, inspect, etc.)
- Emit metrics and events

**State:**

```rust
struct ExtensionManager {
    hosts: HashMap<String, ActorRef<dyn ExtensionHost>>,
    extensions: HashMap<ExtensionId, ExtensionMetadata>,
    registry: HashMap<String, ExtensionId>,
    event_bus: ActorRef<PubSub>,
}
```

### Extension Host

**Type**: Kameo Actor (one per protocol type)  
**Lifecycle**: Created on first use, persists until shutdown

**Responsibilities:**

- Manage lifecycle of extensions using specific protocol
- Translate Morphir operations to protocol operations
- Handle protocol-specific failures and retries
- Monitor extension health
- Report metrics

**Common Interface:**

```rust
#[async_trait]
pub trait ExtensionHost: Send + 'static {
    fn protocol(&self) -> &str;
    async fn initialize(&mut self) -> Result<(), ExtensionError>;
    async fn load_extension(&mut self, config: ExtensionConfig)
        -> Result<ExtensionId, ExtensionError>;
    async fn call(&mut self, id: ExtensionId, method: &str, params: Value)
        -> Result<Value, ExtensionError>;
    async fn unload_extension(&mut self, id: ExtensionId)
        -> Result<(), ExtensionError>;
    async fn capabilities(&self, id: ExtensionId)
        -> Result<Vec<Capability>, ExtensionError>;
}
```

### Extension

**Type**: External process, WASM module, or network service  
**Lifecycle**: Managed by host

**Responsibilities:**

- Implement extension protocol
- Respond to initialization
- Report capabilities
- Handle method calls
- Clean up resources on shutdown

## Data Flow

### Extension Loading

```
User/Config → Manager.load_extension(config)
  ↓
Manager determines protocol from config.source
  ↓
Manager routes to appropriate Host
  ↓
Host spawns/connects to extension
  ↓
Host sends initialize(config)
  ↓
Extension responds with status
  ↓
Host queries capabilities
  ↓
Extension responds with capability list
  ↓
Host returns ExtensionId to Manager
  ↓
Manager stores metadata and returns ID
```

### Extension Invocation

```
Core calls Manager.call_extension(name, method, params)
  ↓
Manager resolves name → ExtensionId
  ↓
Manager looks up metadata to find protocol
  ↓
Manager routes to appropriate Host
  ↓
Host finds extension by ID
  ↓
Host translates call to protocol format
  ↓
Extension executes method
  ↓
Extension returns result
  ↓
Host translates response back to Value
  ↓
Manager returns result to Core
```

## Concurrency Model

### Actor-Based Isolation

Each component is an independent Kameo actor:

- **No shared mutable state** between components
- **Message passing** for all communication
- **Sequential processing** within each actor (no locks needed)
- **Concurrent execution** across actors

### Mailbox Configuration

- **Manager**: Bounded mailbox (capacity: 256) for backpressure
- **Hosts**: Bounded mailbox (capacity: 64) per host
- **Extension tasks**: Unbounded for flexibility

### Parallelism

- Multiple extensions can execute **concurrently**
- Multiple hosts can process **independently**
- Extension calls within same host are **sequential** (per extension)
- Cross-extension calls are **concurrent**

## Error Handling

### Failure Domains

1. **Extension Failure**: Extension process crashes or returns error
   - Host detects failure
   - Host marks extension as unhealthy
   - Host attempts restart based on policy
   - Returns error to caller

2. **Host Failure**: Host actor panics
   - Supervisor restarts host actor
   - Extensions in that host need reload
   - Manager maintains registry state

3. **Manager Failure**: Manager actor panics
   - System-level supervisor restarts manager
   - Hosts remain operational
   - Registry reconstructed from hosts

### Restart Strategies

```rust
pub enum RestartStrategy {
    /// Never restart on failure
    Never,
    /// Restart immediately, up to N times
    Immediate { max_retries: u32 },
    /// Restart with exponential backoff
    Exponential {
        initial_delay: Duration,
        max_delay: Duration,
        max_retries: u32,
    },
}
```

## Configuration

### Extension Configuration Schema

```toml
[[extensions]]
name = "typescript-generator"
enabled = true
protocol = "jsonrpc"
source = { type = "http", url = "http://localhost:3000" }
permissions = { network = false, filesystem = ["./output"] }
restart = { strategy = "exponential", max_retries = 3 }

[[extensions]]
name = "ir-validator"
enabled = true
protocol = "stdio"
source = { type = "process", command = "python3", args = ["./validator.py"] }
permissions = { network = false, filesystem = [] }

[[extensions]]
name = "wasm-optimizer"
enabled = true
protocol = "extism"
source = { type = "wasm", path = "./optimizer.wasm" }
permissions = { max_memory = "10MB", max_execution_time = "5s" }
```

## Performance Considerations

### Latency Characteristics

| Protocol         | Typical Latency | Best For                         |
| ---------------- | --------------- | -------------------------------- |
| WASM (Extism)    | < 1ms           | CPU-bound transformations        |
| WASM (Component) | < 1ms           | CPU-bound with complex types     |
| gRPC             | 1-5ms           | High-throughput, many calls      |
| Stdio            | 5-20ms          | Simple scripts, infrequent calls |
| JSON-RPC         | 10-50ms         | Networked services, web APIs     |

### Throughput

- **WASM**: 10,000+ calls/sec per extension
- **gRPC**: 1,000+ calls/sec per extension
- **Stdio**: 50-100 calls/sec per extension
- **JSON-RPC**: 20-100 calls/sec per extension

### Memory

- **Manager**: ~1MB base
- **Each Host**: 1-5MB base
- **Stdio Extension**: 10-100MB (process overhead)
- **WASM Extension**: 1-10MB (sandbox + module)
- **JSON-RPC/gRPC**: Network client overhead (~500KB)

## Observability

### Metrics

All components emit metrics via OpenTelemetry:

- `morphir.extension.load.duration_ms`
- `morphir.extension.call.duration_ms`
- `morphir.extension.call.count` (by extension, method, status)
- `morphir.extension.failure.count` (by extension, reason)
- `morphir.host.active_extensions` (by protocol)
- `morphir.host.mailbox.depth` (by host)

### Logging

Structured logging using `tracing`:

```rust
#[instrument(skip(self))]
async fn load_extension(&mut self, config: ExtensionConfig)
    -> Result<ExtensionId, ExtensionError> {
    info!("Loading extension", extension = %config.name);
    // ...
}
```

### Events

Published via PubSub actor:

- `extension.loaded` - Extension successfully loaded
- `extension.failed` - Extension failed to load
- `extension.unloaded` - Extension unloaded
- `extension.call.started` - Method call started
- `extension.call.completed` - Method call completed
- `extension.call.failed` - Method call failed

## Security Model

See [09-security-and-isolation.md](./09-security-and-isolation.md) for details.

**Summary:**

- **Process Isolation**: Each extension in separate process (except WASM)
- **Capability-Based**: Extensions declare required capabilities
- **Sandboxing**: WASM extensions run in strict sandbox
- **Resource Limits**: CPU, memory, and execution time limits
- **Principle of Least Privilege**: Extensions only get what they need

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

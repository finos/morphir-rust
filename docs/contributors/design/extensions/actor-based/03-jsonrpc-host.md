---
layout: default
title: JSON-RPC Extension Host
nav_order: 5
parent: Morphir Extensions
---

# JSON-RPC Extension Host

**Status:** Draft  
**Version:** 0.1.0

## Overview

The JSON-RPC Extension Host manages extensions that communicate via JSON-RPC 2.0 over HTTP. This protocol is ideal for web services, microservices, and extensions running as standalone HTTP servers.

## Protocol

### Transport

- **Transport**: HTTP/HTTPS
- **Format**: JSON-RPC 2.0 specification
- **Library**: `jsonrpsee` (Rust client)

### Message Format

#### Request (Host → Extension)

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "morphir.transform",
  "params": {
    "ir": {...}
  }
}
```

#### Response (Extension → Host)

Success:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "transformed": true
  }
}
```

Error:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32000,
    "message": "Invalid IR structure"
  }
}
```

## Implementation

### State

```rust
pub struct JsonRpcExtensionHost {
    extensions: HashMap<ExtensionId, JsonRpcExtension>,
    next_id: u64,
}

struct JsonRpcExtension {
    name: String,
    client: HttpClient,
    capabilities: Vec<Capability>,
}
```

### Loading Process

```rust
async fn load_extension(&mut self, config: ExtensionConfig) -> Result<ExtensionId, ExtensionError> {
    let ExtensionSource::Http { url } = config.source else {
        return Err(ExtensionError::InvalidSource);
    };

    // Create HTTP client
    let client = HttpClientBuilder::default()
        .build(url)?;

    // Initialize extension
    let _init: Value = client
        .request("initialize", rpc_params![config.config])
        .await?;

    // Query capabilities
    let capabilities: Vec<Capability> = client
        .request("capabilities", rpc_params![])
        .await?;

    let id = ExtensionId(self.next_id);
    self.next_id += 1;

    self.extensions.insert(id, JsonRpcExtension {
        name: config.name,
        client,
        capabilities,
    });

    Ok(id)
}
```

## Extension Implementation (TypeScript Example)

```typescript
import { createServer } from "jayson";

const server = createServer({
  initialize: (params, callback) => {
    console.log("Initialized with config:", params);
    callback(null, { status: "ready" });
  },

  capabilities: (params, callback) => {
    callback(null, [
      {
        name: "transform",
        description: "Transform Morphir IR to TypeScript",
      },
    ]);
  },

  transform: (params, callback) => {
    try {
      const { ir } = params;
      // Transform IR to TypeScript
      const output = generateTypeScript(ir);
      callback(null, { output });
    } catch (error) {
      callback(error);
    }
  },
});

server.http().listen(3000);
```

## Configuration Example

```toml
[[extensions]]
name = "typescript-generator"
enabled = true
protocol = "jsonrpc"

[extensions.source]
type = "http"
url = "http://localhost:3000"

[extensions.permissions]
network = true
filesystem = []
```

## Performance Characteristics

- **Latency**: 10-50ms per call (network overhead)
- **Throughput**: 100-500 calls/sec per extension
- **Startup**: Instant (service already running)

## Best For

✅ Existing web services  
✅ Language ecosystems with good HTTP support  
✅ Distributed deployments  
✅ Services shared across multiple Morphir instances

❌ High-frequency local calls  
❌ Offline/air-gapped environments

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
